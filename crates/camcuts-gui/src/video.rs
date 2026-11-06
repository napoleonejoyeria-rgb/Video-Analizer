//! Decodificación del visor y de las miniaturas, fuera del hilo de la interfaz.
//!
//! Igual que el `FrameServer` de Python: un hilo atiende solo el pedido más reciente (arrastrar la
//! barra nunca acumula búsquedas viejas) y, al reproducir, lee cuadros seguidos a tiempo real.
//! Decodifica con ffmpeg (los proxies de `cache/proxies/*.mp4` tienen GOP corto: buscar es casi
//! instantáneo) sin abrir consolas (CREATE_NO_WINDOW). La conversión de color también se aplica aquí,
//! así la interfaz solo cambia la textura.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::datos::{sin_consola, sin_consola_baja};
use crate::motor::{self, ColorSpec, Graduador};

/// Cómo graduar los cuadros de un clip (la clave para reutilizar el graduador ya preparado).
#[derive(Clone, Debug, PartialEq)]
pub struct Grado {
    pub spec: ColorSpec,
    pub log: bool,
    pub ruta_clip: PathBuf,
}

/// Lo que la interfaz pide al decodificador.
pub enum Pedido {
    /// Mostrar el cuadro de `t` y quedarse quieto.
    Buscar { fuente: PathBuf, t: f64, ancho: u32, alto: u32, grado: Option<Grado> },
    /// Reproducir desde `t` a `fps` cuadros por segundo.
    Reproducir { fuente: PathBuf, t: f64, fps: f64, ancho: u32, alto: u32, grado: Option<Grado> },
    /// Parar la reproducción (el último cuadro queda en pantalla).
    Parar,
}

/// Un cuadro listo para subir como textura.
pub struct Cuadro {
    pub t: f64,
    pub ancho: u32,
    pub alto: u32,
    pub rgb: Vec<u8>,
    /// Número de serie: la interfaz solo sube la textura cuando cambia.
    pub serie: u64,
}

/// Estado compartido entre el hilo decodificador y la interfaz.
#[derive(Default)]
pub struct Salida {
    pub cuadro: Option<Cuadro>,
    /// `true` mientras se reproduce; el hilo lo pone en `false` al llegar al final.
    pub reproduciendo: bool,
    pub error: Option<String>,
    /// Número del último pedido. `Visor::pedir` lo sube y, si es "reproducir", pone `reproduciendo` en
    /// `true` en el acto: así la interfaz no lo ve en `false` mientras ffmpeg arranca. El hilo solo lo
    /// vuelve a `false` si no llegó otro pedido después del que estaba atendiendo.
    pub pedido: u64,
    /// Número de serie de la última búsqueda que no dio ningún cuadro (clip sin conexión y sin proxy,
    /// error de ffmpeg, instante más allá del final). Comparte la numeración con `Cuadro::serie`: la
    /// interfaz quita entonces la imagen anterior (que es de otro clip u otro instante) y avisa.
    /// Publicar un cuadro lo borra, y publicar un fallo borra el cuadro pendiente.
    pub sin_cuadro: Option<u64>,
}

/// El servidor de cuadros del visor.
pub struct Visor {
    tx: Sender<Pedido>,
    pub salida: Arc<Mutex<Salida>>,
}

impl Visor {
    /// Arranca el hilo. `despertar` se llama al publicar un cuadro (pide repintar a egui).
    pub fn nuevo(despertar: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, rx) = channel();
        let salida = Arc::new(Mutex::new(Salida::default()));
        let s2 = salida.clone();
        std::thread::Builder::new()
            .name("visor".into())
            .spawn(move || bucle(rx, s2, despertar))
            .expect("hilo del visor");
        Self { tx, salida }
    }

    pub fn pedir(&self, p: Pedido) {
        if let Ok(mut s) = self.salida.lock() {
            s.pedido += 1;
            match p {
                Pedido::Reproducir { .. } => s.reproduciendo = true,
                _ => s.reproduciendo = false,
            }
        }
        let _ = self.tx.send(p);
    }
}

/// Graduador en caché: prepararlo (p. ej. leer una LUT) no debe repetirse en cada cuadro.
struct CacheGrado {
    clave: Option<Grado>,
    g: Option<Graduador>,
}

impl CacheGrado {
    fn aplicar(&mut self, grado: &Option<Grado>, rgb: &mut [u8], w: u32, h: u32) {
        if &self.clave != grado {
            self.g = grado.as_ref().and_then(|g| motor::graduador(&g.spec, g.log, &g.ruta_clip));
            self.clave = grado.clone();
        }
        if let Some(g) = self.g.as_mut() {
            g(rgb, w, h);
        }
    }
}

/// Reproducción en curso: el proceso de ffmpeg y el reloj para ir a tiempo real.
struct Reproduccion {
    hijo: Child,
    t0: f64,
    fps: f64,
    n: u64,
    ancho: u32,
    alto: u32,
    inicio: Instant,
    grado: Option<Grado>,
}

impl Drop for Reproduccion {
    fn drop(&mut self) {
        let _ = self.hijo.kill();
        let _ = self.hijo.wait();
    }
}

/// Los proxies (`cache/proxies/*.mp4`) ya están grabados derechos, pero el de la ruta GPU conserva la
/// marca de rotación de la cámara (p. ej. 90°). Si ffmpeg la aplicara otra vez, el cuadro de un clip
/// vertical saldría girado y, al escalarlo a la caja vertical, estirado. Para ellos se desactiva.
fn es_proxy(fuente: &Path) -> bool {
    // solo los de la carpeta de resultados (`…\cache\proxies\`): una carpeta "Proxies" del propio editor
    // puede tener clips verticales que sí necesitan su marca de rotación
    let padre = fuente.parent();
    let es = |p: Option<&Path>, nombre: &str| p.and_then(|p| p.file_name()).map_or(false, |n| n.eq_ignore_ascii_case(nombre));
    es(padre, "proxies") && es(padre.and_then(Path::parent), "cache")
}

/// Comando de ffmpeg que entrega cuadros RGB crudos desde `t`.
fn comando(fuente: &Path, t: f64, ancho: u32, alto: u32, fps: Option<f64>, un_cuadro: bool) -> Command {
    let mut c = Command::new("ffmpeg");
    c.args(["-v", "error", "-nostdin"]);
    if es_proxy(fuente) {
        c.arg("-noautorotate");
    }
    c.args(["-ss", &format!("{t:.3}"), "-i"]).arg(fuente);
    // Red de seguridad contra imágenes estiradas: el cuadro se escala SIN deformarlo dentro de
    // ancho×alto y lo que sobre se rellena de negro. Si la medida pedida no coincide con la orientación
    // real del video (rotación mal declarada, metadatos de otro programa…), se ven franjas negras,
    // nunca una imagen aplastada.
    let mut vf = format!(
        "scale={ancho}:{alto}:flags=bilinear:force_original_aspect_ratio=decrease,pad={ancho}:{alto}:(ow-iw)/2:(oh-ih)/2:black"
    );
    if let Some(f) = fps {
        vf.push_str(&format!(",fps={f:.5}"));
    }
    c.args(["-vf", &vf, "-an", "-sn"]);
    if un_cuadro {
        c.args(["-frames:v", "1"]);
    }
    c.args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"]);
    c.stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
    sin_consola(&mut c);
    c
}

/// Lee exactamente un cuadro del proceso (None al final del archivo).
fn leer_cuadro(hijo: &mut Child, bytes: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; bytes];
    hijo.stdout.as_mut()?.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn bucle(rx: Receiver<Pedido>, salida: Arc<Mutex<Salida>>, despertar: impl Fn()) {
    let mut serie = 0u64;
    let mut grado = CacheGrado { clave: None, g: None };
    let mut rep: Option<Reproduccion> = None;
    let mut atendido = 0u64;
    let publicar = |serie: &mut u64, t: f64, w: u32, h: u32, rgb: Vec<u8>| {
        *serie += 1;
        if let Ok(mut s) = salida.lock() {
            s.cuadro = Some(Cuadro { t, ancho: w, alto: h, rgb, serie: *serie });
            s.sin_cuadro = None;
        }
        despertar();
    };
    // el pedido no dio ningún cuadro: se publica el fallo para que la interfaz no deje el anterior
    let publicar_fallo = |serie: &mut u64| {
        *serie += 1;
        if let Ok(mut s) = salida.lock() {
            s.cuadro = None;
            s.sin_cuadro = Some(*serie);
        }
        despertar();
    };
    loop {
        // sin reproducir, esperar; reproduciendo, mirar si llegó algo sin bloquear
        let mut pedido = if rep.is_some() {
            match rx.try_recv() {
                Ok(p) => Some(p),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(p) => Some(p),
                Err(_) => return,
            }
        };
        // solo vale el pedido más reciente
        while let Ok(p) = rx.try_recv() {
            pedido = Some(p);
        }
        // número del pedido que se atiende: si luego llega otro, este hilo no toca `reproduciendo`
        if pedido.is_some() {
            atendido = salida.lock().map(|s| s.pedido).unwrap_or(atendido);
        }
        let marcar_parado = |salida: &Arc<Mutex<Salida>>, atendido: u64| {
            if let Ok(mut s) = salida.lock() {
                if s.pedido == atendido {
                    s.reproduciendo = false;
                }
            }
        };
        match pedido {
            Some(Pedido::Parar) => {
                rep = None;
                marcar_parado(&salida, atendido);
            }
            Some(Pedido::Buscar { fuente, t, ancho, alto, grado: g }) => {
                rep = None;
                marcar_parado(&salida, atendido);
                let bytes = (ancho * alto * 3) as usize;
                let hijo = comando(&fuente, t, ancho, alto, None, true).spawn();
                match hijo {
                    Ok(mut h) => {
                        let cuadro = leer_cuadro(&mut h, bytes);
                        let _ = h.kill();
                        let _ = h.wait();
                        match cuadro {
                            Some(mut rgb) => {
                                grado.aplicar(&g, &mut rgb, ancho, alto);
                                publicar(&mut serie, t, ancho, alto, rgb);
                            }
                            // sin conexión y sin proxy, ffmpeg falló o `t` pasa del final
                            None => publicar_fallo(&mut serie),
                        }
                    }
                    Err(e) => {
                        if let Ok(mut s) = salida.lock() {
                            s.error = Some(format!("No se pudo abrir ffmpeg: {e}"));
                        }
                        publicar_fallo(&mut serie);
                    }
                }
            }
            Some(Pedido::Reproducir { fuente, t, fps, ancho, alto, grado: g }) => {
                rep = None;
                let fps = fps.clamp(1.0, 30.0);
                match comando(&fuente, t, ancho, alto, Some(fps), false).spawn() {
                    Ok(hijo) => rep = Some(Reproduccion { hijo, t0: t, fps, n: 0, ancho, alto, inicio: Instant::now(), grado: g }),
                    Err(e) => {
                        marcar_parado(&salida, atendido);
                        if let Ok(mut s) = salida.lock() {
                            s.error = Some(format!("No se pudo abrir ffmpeg: {e}"));
                        }
                        publicar_fallo(&mut serie);
                    }
                }
            }
            None => {}
        }
        // un cuadro más de la reproducción, a su hora
        let mut terminar = false;
        let mut sin_ninguno = false;
        if let Some(r) = rep.as_mut() {
            let bytes = (r.ancho * r.alto * 3) as usize;
            match leer_cuadro(&mut r.hijo, bytes) {
                Some(mut rgb) => {
                    let g = r.grado.clone();
                    grado.aplicar(&g, &mut rgb, r.ancho, r.alto);
                    let t = r.t0 + r.n as f64 / r.fps;
                    let debido = r.inicio + Duration::from_secs_f64(r.n as f64 / r.fps);
                    let ahora = Instant::now();
                    if debido > ahora {
                        std::thread::sleep(debido - ahora);
                    }
                    r.n += 1;
                    publicar(&mut serie, t, r.ancho, r.alto, rgb);
                }
                None => {
                    terminar = true;
                    // la reproducción no dio ni un cuadro: igual que una búsqueda fallida
                    sin_ninguno = r.n == 0;
                }
            }
        }
        if terminar {
            rep = None;
            marcar_parado(&salida, atendido);
            if sin_ninguno {
                publicar_fallo(&mut serie);
            }
            despertar();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Miniaturas de la lista de clips
// ---------------------------------------------------------------------------------------------

/// Tamaño de las miniaturas (el doble de los 52×36 de la maqueta, para pantallas con escala).
pub const MINI_W: u32 = 156;
pub const MINI_H: u32 = 108;

/// Pedido de miniatura: clave del clip, video del que sacarla, instante y color.
pub struct PedidoMini {
    pub clave: String,
    pub fuente: PathBuf,
    pub t: f64,
    pub grado: Option<Grado>,
}

/// Hilo de miniaturas: un ffmpeg a la vez, con prioridad baja.
pub struct Miniaturas {
    tx: Sender<PedidoMini>,
    pub listas: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
}

impl Miniaturas {
    pub fn nuevo(despertar: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx, rx) = channel::<PedidoMini>();
        let listas = Arc::new(Mutex::new(Vec::new()));
        let l2 = listas.clone();
        std::thread::Builder::new()
            .name("miniaturas".into())
            .spawn(move || {
                let mut grado = CacheGrado { clave: None, g: None };
                while let Ok(p) = rx.recv() {
                    let mut c = Command::new("ffmpeg");
                    c.args(["-v", "error", "-nostdin"]);
                    if es_proxy(&p.fuente) {
                        c.arg("-noautorotate"); // el proxy ya está derecho (ver `es_proxy`)
                    }
                    c.args(["-ss", &format!("{:.3}", p.t), "-i"]).arg(&p.fuente);
                    c.args([
                        "-vf",
                        &format!("scale={MINI_W}:{MINI_H}:force_original_aspect_ratio=increase,crop={MINI_W}:{MINI_H}"),
                        "-frames:v",
                        "1",
                        "-an",
                        "-f",
                        "rawvideo",
                        "-pix_fmt",
                        "rgb24",
                        "pipe:1",
                    ]);
                    c.stdout(Stdio::piped()).stderr(Stdio::null()).stdin(Stdio::null());
                    sin_consola_baja(&mut c);
                    let Ok(mut h) = c.spawn() else { continue };
                    let cuadro = leer_cuadro(&mut h, (MINI_W * MINI_H * 3) as usize);
                    let _ = h.wait();
                    if let Some(mut rgb) = cuadro {
                        grado.aplicar(&p.grado, &mut rgb, MINI_W, MINI_H);
                        if let Ok(mut l) = l2.lock() {
                            l.push((p.clave, rgb));
                        }
                        despertar();
                    }
                }
            })
            .expect("hilo de miniaturas");
        Self { tx, listas }
    }

    pub fn pedir(&self, p: PedidoMini) {
        let _ = self.tx.send(p);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Solo los proxies de la carpeta de resultados se leen sin girar; la carpeta "Proxies" del editor no.
    #[test]
    fn solo_proxies_de_resultados() {
        assert!(es_proxy(Path::new("D:\\boda\\CAMCUTS\\cache\\proxies\\C1_ab12cd34.mp4")));
        assert!(!es_proxy(Path::new("D:\\boda\\Proxies\\IMG_0001.MOV")));
        assert!(!es_proxy(Path::new("D:\\boda\\proxies\\IMG_0001.MOV")));
    }

    /// Una búsqueda que no decodifica nada (aquí: un clip que no existe, como uno sin conexión y sin
    /// proxy) publica el fallo y quita el cuadro pendiente, en vez de dejar el anterior en pantalla.
    #[test]
    fn busqueda_sin_cuadro_publica_el_fallo() {
        let v = Visor::nuevo(|| {});
        if let Ok(mut s) = v.salida.lock() {
            // un cuadro viejo de otro clip que aún no se recogió
            s.cuadro = Some(Cuadro { t: 0.0, ancho: 2, alto: 2, rgb: vec![0; 12], serie: 0 });
        }
        let fuente = std::env::temp_dir().join("camcuts_prueba_visor").join("no existe.mp4");
        v.pedir(Pedido::Buscar { fuente, t: 1.0, ancho: 16, alto: 16, grado: None });
        let inicio = Instant::now();
        loop {
            let s = v.salida.lock().unwrap();
            if s.sin_cuadro.is_some() {
                assert!(s.cuadro.is_none(), "el cuadro anterior no debe quedar pendiente");
                break;
            }
            drop(s);
            assert!(inicio.elapsed() < Duration::from_secs(20), "la búsqueda fallida no se publicó");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Cuadro real del visor con "Automático": `CAMCUTS_VISOR_CLIP` (original, con su XML de cámara),
    /// `CAMCUTS_VISOR_FUENTE` (lo que se decodifica: su proxy) y `CAMCUTS_VISOR_PNG` (salida). Sin esas
    /// variables no hace nada. Un S-Log3 debe salir convertido a Rec.709, no gris.
    #[test]
    fn cuadro_real_con_color_automatico() {
        let (Ok(clip), Ok(fuente), Ok(png)) = (std::env::var("CAMCUTS_VISOR_CLIP"), std::env::var("CAMCUTS_VISOR_FUENTE"), std::env::var("CAMCUTS_VISOR_PNG")) else {
            return;
        };
        let (ancho, alto) = (540u32, 960u32);
        let mut cuadros = Vec::new();
        for grado in [None, Some(Grado { spec: crate::motor::ColorSpec::auto(), log: true, ruta_clip: PathBuf::from(&clip) })] {
            let v = Visor::nuevo(|| {});
            v.pedir(Pedido::Buscar { fuente: PathBuf::from(&fuente), t: 25.0, ancho, alto, grado });
            let inicio = Instant::now();
            let c = loop {
                if let Some(c) = v.salida.lock().unwrap().cuadro.take() {
                    break c;
                }
                assert!(inicio.elapsed() < Duration::from_secs(30), "el visor no dio cuadro");
                std::thread::sleep(Duration::from_millis(20));
            };
            let sat = c.rgb.chunks_exact(3).map(|p| (*p.iter().max().unwrap() as f64) - (*p.iter().min().unwrap() as f64)).sum::<f64>() / (ancho * alto) as f64;
            println!("saturación media {sat:.1}");
            cuadros.push((c.rgb, sat));
        }
        // convertido: bastante más color que el S-Log3 tal cual (medido: 13.6 → 53.5)
        assert!(cuadros[1].1 > cuadros[0].1 * 2.5, "el color automático no convirtió el S-Log3");
        let mut tira = image::RgbImage::new(ancho * 2, alto);
        for (i, (rgb, _)) in cuadros.iter().enumerate() {
            let img = image::RgbImage::from_raw(ancho, alto, rgb.clone()).unwrap();
            image::imageops::replace(&mut tira, &img, (i as u32 * ancho) as i64, 0);
        }
        tira.save(&png).unwrap();
    }

    /// "Reproducir" se marca en el acto al pedirlo (antes, un repintado entre el pedido y el arranque de
    /// ffmpeg lo borraba: el cabezal se congelaba y Espacio volvía a empezar en vez de pausar).
    #[test]
    fn reproducir_se_marca_al_pedir() {
        // un visor sin hilo: nadie atiende los pedidos, así la prueba no depende de tiempos
        let (tx, _rx) = channel();
        let v = Visor { tx, salida: Arc::new(Mutex::new(Salida::default())) };
        v.pedir(Pedido::Reproducir { fuente: PathBuf::from("x.mp4"), t: 0.0, fps: 25.0, ancho: 16, alto: 16, grado: None });
        assert!(v.salida.lock().unwrap().reproduciendo);
        let n = v.salida.lock().unwrap().pedido;
        v.pedir(Pedido::Parar);
        let s = v.salida.lock().unwrap();
        assert!(!s.reproduciendo);
        assert_eq!(s.pedido, n + 1);
    }
}