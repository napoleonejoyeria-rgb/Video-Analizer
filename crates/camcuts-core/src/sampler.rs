//! Muestreo de cuadros con ffmpeg (port de `sampler.py`): N muestras por segundo, a 960 px en el lado largo.
//!
//! ffmpeg corre como subproceso (sin ventana de consola: `CREATE_NO_WINDOW`) y entrega BGR crudo por
//! una tubería. Primero se intenta decodificar y escalar en la GPU (NVDEC + `scale_cuda`, también el
//! H.264 4:2:2 de 10 bits de Sony); si no da ni un cuadro, se repite por CPU. La misma pasada puede
//! escribir el proxy de vista previa para la app.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::pyfmt::float_repr;
use crate::types::Frame;

/// Lado largo de los cuadros de análisis.
pub const LONG_EDGE: u32 = 960;
/// Lado largo de la copia de detalle (segunda mirada en orejas, cuello y muñecas).
pub const DETAIL_EDGE: u32 = 1920;
/// El escalado de la GPU (`scale_cuda` lanczos) mide ~10 % más nítido que el "area" de CPU con el
/// que se calibraron los umbrales (razón 1.103 en dos clips distintos, correlación 0.94-0.98).
pub const GPU_LAP_SCALE: f64 = 1.0 / 1.103;

/// Contenedores que ffmpeg decodifica directamente.
pub const FFMPEG_EXT: &[&str] = &[".mp4", ".mov", ".mxf", ".m4v", ".mts", ".m2ts", ".avi", ".mkv", ".webm", ".3gp", ".dng", ".crm"];
/// RAW de cámara que ffmpeg no decodifica: se analiza un proxy y se exporta apuntando al RAW.
pub const RAW_EXT: &[&str] = &[".braw", ".r3d", ".nev", ".ari", ".arx", ".mxr", ".nraw"];
/// Carpetas habituales de proxies.
pub const PROXY_DIR_NAMES: &[&str] = &["Proxy", "Proxies", "proxy", "proxies", "ProxyMedia", "Proxy Media"];

/// Proxy de vista previa: todos los cuadros a tamaño de análisis, GOP corto para buscar al instante.
///
/// `veryfast` + CRF 20, sin `-tune fastdecode`: `ultrafast`/`fastdecode` apagan el filtro antibloques y
/// CABAC, y con CRF 28 la piel y los bordes de la joya salían en bloques y borrosos (2026-11-05). Medido
/// en 10 s de C9262 a 540×960: 0.41 s de CPU (~730 cuadros/s, sigue sin frenar el análisis) y el
/// archivo pasa de ~4.3 a ~7.4 MB por minuto.
pub const PROXY_ARGS: &[&str] = &["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-g", "8", "-bf", "0", "-pix_fmt", "yuv420p", "-an", "-movflags", "+faststart"];

/// Extensión en minúsculas con punto (".mp4"), o vacío.
pub fn suffix_lower(p: &Path) -> String {
    p.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default()
}

/// ¿Es una copia de metadatos de macOS ("._C0001.MP4", la que deja en discos exFAT)? Tiene extensión de
/// video pero no lo es: no se lista ni se analiza (daba "error" en la lista de la ventana).
pub fn es_basura_mac(p: &Path) -> bool {
    p.file_name().map(|n| n.to_string_lossy().starts_with("._")).unwrap_or(false)
}

/// ¿Es un video que CAMCUTS sabe analizar (directo o por proxy)?
pub fn is_video(p: &Path) -> bool {
    let s = suffix_lower(p);
    FFMPEG_EXT.contains(&s.as_str()) || RAW_EXT.contains(&s.as_str())
}

/// Ejecuta un programa sin ventana y devuelve su salida estándar (error si termina mal).
pub fn run_quiet(program: &str, args: &[&str]) -> Result<Vec<u8>> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    no_window(&mut cmd);
    let out = cmd.output().with_context(|| format!("no se pudo ejecutar {program}"))?;
    if !out.status.success() {
        return Err(anyhow!("{program} terminó con error: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

/// Oculta la ventana de consola de los subprocesos en Windows.
pub fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Fracción reducida (frecuencia de cuadro exacta, como `fractions.Fraction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fraction {
    pub num: i64,
    pub den: i64,
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

impl Fraction {
    /// Fracción reducida con denominador positivo.
    pub fn new(num: i64, den: i64) -> Self {
        let g = gcd(num, den).max(1);
        let s = if den < 0 { -1 } else { 1 };
        Fraction { num: s * num / g, den: s * den / g }
    }
    /// Lee "30000/1001" o "25".
    pub fn parse(s: &str) -> Result<Self> {
        let (a, b) = s.split_once('/').unwrap_or((s, "1"));
        let (a, b): (i64, i64) = (a.trim().parse()?, b.trim().parse()?);
        if b == 0 {
            return Err(anyhow!("fracción con denominador 0: {s}"));
        }
        Ok(Fraction::new(a, b))
    }
    pub fn as_f64(&self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// Datos de un clip (`ClipInfo` de Python).
#[derive(Debug, Clone, PartialEq)]
pub struct ClipInfo {
    /// Medio original: a lo que apunta la línea de tiempo exportada.
    pub path: PathBuf,
    /// Ancho mostrado tras la rotación.
    pub width: u32,
    pub height: u32,
    /// Frecuencia de cuadro de la fuente.
    pub fps: Fraction,
    /// Duración en segundos.
    pub duration: f64,
    /// Proxy decodificado de verdad, cuando el original es un RAW que ffmpeg no lee.
    pub decode_path: Option<PathBuf>,
    /// Timecode de inicio de cámara; ";" antes de los cuadros = drop-frame.
    pub start_tc: String,
    /// Rotación de la matriz de display tal como la da ffprobe.
    pub rotation: i32,
    /// H.264: cuando no se escribe proxy, el análisis puede decodificar solo los cuadros I/P (ver
    /// [`SALTAR_B`]).
    pub saltar_b: bool,
}

impl ClipInfo {
    pub fn frame_count(&self) -> i64 {
        (self.duration * self.fps.as_f64()).round_ties_even() as i64
    }
    /// Lo que se decodifica (el proxy si lo hay).
    pub fn source(&self) -> &Path {
        self.decode_path.as_deref().unwrap_or(&self.path)
    }
}

/// Un proxy comparte el nombre del original: se busca al lado, en carpetas de proxy y en `extra_dirs`.
pub fn find_proxy(original: &Path, extra_dirs: &[PathBuf]) -> Option<PathBuf> {
    let parent = original.parent().unwrap_or(Path::new(""));
    let stem = original.file_stem()?.to_string_lossy().to_string();
    let mut dirs: Vec<(PathBuf, bool)> = vec![(parent.to_path_buf(), false)];
    dirs.extend(PROXY_DIR_NAMES.iter().map(|n| (parent.join(n), false)));
    dirs.extend(extra_dirs.iter().map(|d| (d.clone(), true)));
    for (d, recursive) in dirs {
        if !d.is_dir() {
            continue;
        }
        let mut found = Vec::new();
        collect_stem(&d, &stem, recursive, &mut found);
        for q in found {
            if q != original && FFMPEG_EXT.contains(&suffix_lower(&q).as_str()) {
                return Some(q);
            }
        }
    }
    None
}

fn collect_stem(dir: &Path, stem: &str, recursive: bool, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if recursive {
                collect_stem(&p, stem, true, out);
            }
        } else if p.file_name().map(|n| n.to_string_lossy().starts_with(&format!("{stem}."))).unwrap_or(false) {
            out.push(p);
        }
    }
}

/// ffprobe completo (JSON) de un archivo, con un motivo claro si no se puede leer (vacío, dañado).
fn probe_json(path: &Path) -> Result<Value> {
    match std::fs::metadata(path) {
        Err(e) => return Err(anyhow!("no se puede abrir: {e}")),
        Ok(m) if m.len() == 0 => return Err(anyhow!("archivo vacío (0 bytes)")),
        Ok(_) => {}
    }
    let out = run_quiet("ffprobe", &["-v", "error", "-print_format", "json", "-show_streams", "-show_format", &path.to_string_lossy()])
        .map_err(|e| anyhow!("archivo dañado o incompleto ({})", short_error(&e.to_string())))?;
    Ok(serde_json::from_slice(&out)?)
}

/// Códec de la primera pista de video de un ffprobe, o por qué no hay una que ffmpeg decodifique.
fn codec_of(data: &Value) -> Result<String> {
    let s = data["streams"].as_array().into_iter().flatten().find(|st| st["codec_type"] == "video");
    let codec = s.and_then(|s| s["codec_name"].as_str()).unwrap_or("");
    match (s, codec) {
        (None, _) => Err(anyhow!("sin pista de video (¿solo audio?)")),
        (Some(_), "" | "none") => Err(anyhow!("códec de video que ffmpeg no decodifica")),
        _ => Ok(codec.to_string()),
    }
}

/// Códec de la pista de video, o por qué no se puede leer (archivo vacío, dañado, solo audio…).
pub fn video_codec(path: &Path) -> Result<String> {
    codec_of(&probe_json(path)?)
}

/// Última línea útil de un error de ffmpeg/ffprobe, sin la ruta del archivo (que ya se muestra aparte).
fn short_error(msg: &str) -> String {
    let line = msg.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or(msg);
    let line = line.rsplit_once(": ").map(|(_, b)| b).unwrap_or(line);
    line.chars().take(120).collect()
}

/// ¿ffmpeg puede decodificar la pista de video?
pub fn decodable(path: &Path) -> bool {
    video_codec(path).is_ok()
}

/// Analiza un clip; para RAW (o un video que ffmpeg no lee) busca un proxy. `None` = es un RAW y hace
/// falta un proxy que no se encontró; un video normal que no se puede leer (vacío, dañado, solo audio)
/// es un error con el motivo, no un "RAW sin proxy".
pub fn open_clip(path: &Path, proxy_dirs: &[PathBuf]) -> Result<Option<ClipInfo>> {
    let raw = RAW_EXT.contains(&suffix_lower(path).as_str());
    let mut why = None;
    if !raw {
        // un solo ffprobe (~0.3 s en un 4K de Sony) sirve para saber si se decodifica y para los datos
        match probe_json(path).and_then(|d| codec_of(&d).map(|_| d)) {
            Ok(data) => return info_from_json(path, &data).map(Some),
            Err(e) => why = Some(e),
        }
    }
    let Some(proxy) = find_proxy(path, proxy_dirs) else {
        return match why {
            Some(e) => Err(e),
            None => Ok(None),
        };
    };
    let mut info = probe(&proxy)?;
    info.decode_path = Some(proxy);
    info.path = path.to_path_buf();
    Ok(Some(info))
}

/// ffprobe de un clip: tamaño mostrado, fps, duración, timecode y rotación.
pub fn probe(path: &Path) -> Result<ClipInfo> {
    let out = run_quiet("ffprobe", &["-v", "error", "-print_format", "json", "-show_streams", "-show_format", &path.to_string_lossy()])?;
    let data: Value = serde_json::from_slice(&out)?;
    info_from_json(path, &data)
}

/// [`ClipInfo`] a partir de la salida JSON de ffprobe.
fn info_from_json(path: &Path, data: &Value) -> Result<ClipInfo> {
    let streams = data["streams"].as_array().cloned().unwrap_or_default();
    let s = streams.iter().find(|st| st["codec_type"] == "video").ok_or_else(|| anyhow!("{}: sin pista de video", path.display()))?;
    let num = |v: &Value| v.as_i64().or_else(|| v.as_str().and_then(|x| x.parse().ok()));
    let (mut w, mut h) = (num(&s["width"]).unwrap_or(0) as u32, num(&s["height"]).unwrap_or(0) as u32);
    let mut rotation = 0i32;
    for sd in s["side_data_list"].as_array().into_iter().flatten() {
        if let Some(r) = num(&sd["rotation"]) {
            rotation = r as i32;
        }
    }
    if rotation.abs() == 90 || rotation.abs() == 270 {
        std::mem::swap(&mut w, &mut h);
    }
    // `float(s.get("duration") or data["format"]["duration"])`: el texto de la pista si no está vacío
    let fdur = |v: &Value| v.as_str().filter(|x| !x.is_empty()).and_then(|x| x.parse::<f64>().ok()).or(v.as_f64());
    let duration = fdur(&s["duration"]).or_else(|| fdur(&data["format"]["duration"])).ok_or_else(|| anyhow!("{}: sin duración", path.display()))?;
    // r_frame_rate "0/0" (pistas raras) → la frecuencia media; sin ninguna, error claro
    let fps = Fraction::parse(s["r_frame_rate"].as_str().unwrap_or("0/1"))
        .ok()
        .filter(|f| f.num > 0)
        .or_else(|| Fraction::parse(s["avg_frame_rate"].as_str().unwrap_or("0/1")).ok().filter(|f| f.num > 0))
        .ok_or_else(|| anyhow!("{}: sin frecuencia de cuadro", path.display()))?;
    if w == 0 || h == 0 {
        return Err(anyhow!("{}: pista de video sin tamaño", path.display()));
    }
    let mut start_tc = start_timecode(&data, fps);
    // Sony dice en su XML si el timecode es drop-frame (bit 0x40 del byte de cuadros); manda sobre la
    // suposición de `start_timecode` (cámara configurada en NDF a 29.97/59.94)
    if sony_drop_frame(path) == Some(false) {
        start_tc = start_tc.replace(';', ":");
    }
    let saltar_b = s["codec_name"].as_str() == Some("h264");
    Ok(ClipInfo { path: path.to_path_buf(), width: w, height: h, fps, duration, decode_path: None, start_tc, rotation, saltar_b })
}

/// Marca de drop-frame del primer `LtcChange` del XML de Sony, si lo hay: el `<stem>M01.XML` de al lado
/// o, si falta, el que la cámara graba dentro del MP4 (el mismo que usa el color, `color::sony_xml`).
///
/// El valor son 4 bytes BCD (cuadros, segundos, minutos, horas); el bit 0x40 del de cuadros es DF.
pub fn sony_drop_frame(video: &Path) -> Option<bool> {
    let xml = crate::color::texto_xml_sony(video)?;
    let at = xml.find("<LtcChange ")?;
    let rest = &xml[at..];
    let v = rest.find("value=\"")? + 7;
    let byte = u8::from_str_radix(rest.get(v..v + 2)?, 16).ok()?;
    Some(byte & 0x40 != 0)
}

/// ¿Esta frecuencia tiene timecode drop-frame? Solo las NTSC múltiplos de 29.97 (29.97, 59.94,
/// 119.88): a 23.976 no existe el drop-frame aunque el denominador también sea 1001.
pub fn drop_frame_rate(fps: Fraction) -> bool {
    fps.den == 1001 && fps.num % 30000 == 0
}

/// Timecode de inicio de una pista de timecode (separador fiable) o de los metadatos.
///
/// El timecode de los metadatos de cámara (Sony XAVC) no trae marca de drop-frame; a 29.97/59.94 las
/// cámaras profesionales graban en drop-frame por defecto, que es también lo que asume Resolve.
pub fn start_timecode(data: &Value, fps: Fraction) -> String {
    let mut tc = String::new();
    for st in data["streams"].as_array().into_iter().flatten() {
        let stc = st["tags"]["timecode"].as_str().unwrap_or("");
        if !stc.is_empty() && st["codec_tag_string"] == "tmcd" {
            return stc.to_string(); // pista de timecode de verdad
        }
        if tc.is_empty() {
            tc = stc.to_string(); // p. ej. pista "rtmd" de Sony: sin información de drop-frame
        }
    }
    if tc.is_empty() {
        tc = data["format"]["tags"]["timecode"].as_str().unwrap_or("").to_string();
    }
    if !tc.is_empty() && drop_frame_rate(fps) && !tc.contains(';') && tc.len() >= 9 && tc.is_char_boundary(8) && tc.is_char_boundary(9) {
        tc = format!("{};{}", &tc[..8], &tc[9..]);
    }
    tc
}

/// Timecode de inicio como número de cuadros a la frecuencia del clip (0 sin timecode).
pub fn start_frames(info: &ClipInfo) -> f64 {
    if info.start_tc.is_empty() {
        return 0.0;
    }
    let tc = &info.start_tc;
    let parts: Vec<i64> = tc.replace([';', '.'], ":").split(':').filter_map(|x| x.trim().parse().ok()).collect();
    if parts.len() != 4 {
        return 0.0;
    }
    let (h, m, s, f) = (parts[0], parts[1], parts[2], parts[3]);
    let nominal = info.fps.as_f64().round_ties_even() as i64;
    let mut frames = (h * 3600 + m * 60 + s) * nominal + f;
    if tc.contains(';') && drop_frame_rate(info.fps) {
        // drop-frame SMPTE: se saltan 2 (29.97) o 4 (59.94) números cada minuto salvo cada 10
        let minutes = h * 60 + m;
        frames -= (nominal / 15) * (minutes - minutes / 10);
    }
    frames as f64
}

/// Tamaño de análisis (lado largo 960, dimensiones pares).
pub fn scaled_size(info: &ClipInfo) -> (u32, u32) {
    let scale = LONG_EDGE as f64 / info.width.max(info.height) as f64;
    let even = |v: u32| ((v as f64 * scale / 2.0).round_ties_even() as u32) * 2;
    (even(info.width), even(info.height))
}

/// Tamaño de la copia de detalle (hasta 1920 px).
pub fn detail_size(info: &ClipInfo) -> (u32, u32) {
    let scale = (DETAIL_EDGE as f64 / info.width.max(info.height) as f64).min(1.0);
    let even = |v: u32| ((v as f64 * scale / 2.0).round_ties_even() as u32) * 2;
    (even(info.width), even(info.height))
}

/// Pasos de `transpose_cuda` que reproducen la autorrotación de ffmpeg (`None` = no soportada).
fn transpose(rotation: i32) -> Option<Vec<String>> {
    match (-rotation).rem_euclid(360) {
        0 => Some(vec![]),
        90 => Some(vec!["transpose_cuda=dir=clock".into()]),
        270 => Some(vec!["transpose_cuda=dir=cclock".into()]),
        _ => None,
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Inicio de un comando de GPU hasta la ruta de entrada incluida.
///
/// Con `-noautorotate` ffmpeg no gira los cuadros (lo hace `transpose_cuda`), pero sí copiaba la matriz
/// de display de la cámara al proxy: un proxy ya derecho (540x960) llevaba aún "rotation=90" y los
/// reproductores que la respetan lo mostraban girado o estirado. `-display_rotation 0` en la entrada
/// reemplaza esa matriz y el proxy sale sin ninguna (comprobado con ffprobe en ffmpeg 9).
fn gpu_input(info: &ClipInfo, saltar: bool) -> Vec<String> {
    let mut c = s(&["ffmpeg", "-v", "error", "-y", "-noautorotate"]);
    if info.rotation != 0 {
        c.extend(s(&["-display_rotation", "0"]));
    }
    if saltar {
        c.extend(s(&["-skip_frame", SALTAR_CUADROS]));
    }
    c.extend(s(&["-hwaccel", "cuda", "-hwaccel_output_format", "cuda", "-i"]));
    c.push(info.source().to_string_lossy().to_string());
    c
}

/// Qué cuadros se dejan sin decodificar al reanalizar (ver [`saltar_b`]): `noref`, los que ningún otro
/// usa de referencia. No `bidir`: ese descarta TODAS las porciones B, también las B de referencia que
/// x264 (b-pyramid normal, su valor por defecto) y otras cámaras dejan que un P use, y esos P salían con
/// errores ocultados sin ningún aviso. En el material de Sony las B no son de referencia y ahorra lo mismo
/// (20 s de C9262 en NVDEC: 1.36 s con las dos, 2.5 s sin saltar; medido el 2026-11-06).
const SALTAR_CUADROS: &str = "noref";

/// Decodificar solo los cuadros que alguien usa de referencia (`-skip_frame noref`, en el material de
/// Sony los I/P) cuando el análisis no escribe proxy (ya existe uno al día, o `--no-proxy`). Solo en
/// H.264: en el material de Sony (GOP I B B P …, 15 cuadros) el decodificador hace un tercio del trabajo y
/// va ~2 veces más rápido, y es justo lo que frena el análisis con el detector propio. Las muestras salen
/// del cuadro decodificado más cercano (a ≤ 50 ms del instante de siempre). Los cuadros que sí se
/// decodifican salen idénticos a una decodificación completa (prueba `robustez::saltar_no_referencia_*`).
/// `CAMCUTS_SALTAR_B=0` lo apaga (pruebas de comparación).
pub fn saltar_b(info: &ClipInfo, part: Option<&Path>) -> bool {
    info.saltar_b && part.is_none() && std::env::var("CAMCUTS_SALTAR_B").map(|v| v != "0").unwrap_or(true)
}

/// ¿Ya hay un proxy de vista previa con los ajustes de ahora ([`PROXY_ARGS`])? Se mira la firma de x264
/// al principio del archivo (`crf=20.0`, filtro antibloques encendido). Los de antes (`ultrafast`,
/// CRF 28, sin antibloques) se vuelven a hacer al reanalizar el clip.
pub fn proxy_vigente(proxy: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(proxy) else { return false };
    // La firma va en el primer cuadro, al principio de la caja `mdat`. Con `+faststart` el `moov` va
    // delante y crece ~4.5 B por cuadro: en un proxy de más de ~32 min a 59.94 (o ~64 min a 29.97) ya no
    // cabía en los primeros 512 KB y el proxy se reescribía en cada reanálisis. Se busca primero al
    // principio de `mdat` y, si el archivo no tiene esa estructura, como antes.
    if let Some(inicio) = carga_mdat(&mut f) {
        let mut buf = Vec::with_capacity(PROXY_MDAT_BYTES);
        if f.seek(SeekFrom::Start(inicio)).is_ok() && (&mut f).take(PROXY_MDAT_BYTES as u64).read_to_end(&mut buf).is_ok() && firma_x264(&buf) {
            return true;
        }
    }
    let mut buf = Vec::with_capacity(PROXY_FIRMA_BYTES);
    if f.seek(SeekFrom::Start(0)).is_err() || f.take(PROXY_FIRMA_BYTES as u64).read_to_end(&mut buf).is_err() {
        return false;
    }
    firma_x264(&buf)
}

/// ¿Lleva `buf` la firma de x264 de los proxies de ahora?
fn firma_x264(buf: &[u8]) -> bool {
    // en bytes: el archivo es binario y cortar un texto "con pérdidas" a mitad de un carácter entra en pánico
    let busca = |h: &[u8], a: &[u8]| h.windows(a.len()).position(|w| w == a);
    let Some(i) = busca(buf, b"x264 - core") else { return false };
    let opciones = &buf[i..buf.len().min(i + 2000)];
    busca(opciones, PROXY_FIRMA.as_bytes()).is_some() && busca(opciones, b"deblock=1").is_some()
}

/// Posición donde empiezan los datos de la caja `mdat` de primer nivel de un MP4, recorriendo las cajas
/// (tamaño de 32 bits, o de 64 si vale 1). `None` si no hay o el archivo no tiene esa estructura.
fn carga_mdat(f: &mut std::fs::File) -> Option<u64> {
    use std::io::{Read, Seek, SeekFrom};
    let len = f.metadata().ok()?.len();
    let mut pos = 0u64;
    // pocas cajas de primer nivel en un MP4 (ftyp, free, moov, mdat…): un tope por si el archivo es basura
    for _ in 0..64 {
        if pos + 8 > len {
            return None;
        }
        f.seek(SeekFrom::Start(pos)).ok()?;
        let mut cab = [0u8; 16];
        f.read_exact(&mut cab[..8]).ok()?;
        let mut tam = u32::from_be_bytes([cab[0], cab[1], cab[2], cab[3]]) as u64;
        let mut cabecera = 8u64;
        if tam == 1 {
            f.read_exact(&mut cab[8..16]).ok()?;
            tam = u64::from_be_bytes(cab[8..16].try_into().ok()?);
            cabecera = 16;
        }
        if &cab[4..8] == b"mdat" {
            return Some(pos + cabecera);
        }
        // 0 = hasta el final (y no es mdat); menos que la cabecera = no es un MP4
        if tam < cabecera {
            return None;
        }
        pos = pos.checked_add(tam)?;
    }
    None
}

/// Firma de x264 de los proxies de ahora (ver [`PROXY_ARGS`]).
const PROXY_FIRMA: &str = "crf=20.0";
/// Bytes del principio del proxy donde se busca la firma (con `+faststart` el `moov` va delante).
const PROXY_FIRMA_BYTES: usize = 512 * 1024;
/// Bytes del principio de `mdat` donde se busca la firma (el primer cuadro la lleva delante).
const PROXY_MDAT_BYTES: usize = 64 * 1024;

/// Comando de GPU (NVDEC + scale_cuda), opcionalmente escribiendo el proxy en la misma pasada.
///
/// El proxy se codifica con x264 `ultrafast` en la CPU: a 960 px cuesta ~0.3 ms por cuadro y ffmpeg lo
/// termina ~8 ms después del último cuadro (medido en Octubre 27), así que no frena el análisis.
pub fn gpu_cmd(info: &ClipInfo, per_second: f64, size: (u32, u32), proxy: Option<&Path>) -> Option<Vec<String>> {
    let steps = transpose(info.rotation)?;
    let (w, h) = size;
    let pre = if steps.is_empty() { (w, h) } else { (h, w) }; // escalar antes de rotar: menos píxeles
    let mut chain = vec![format!("scale_cuda={}:{}:format=nv12:interp_algo=lanczos", pre.0, pre.1)];
    chain.extend(steps);
    let mut cmd = gpu_input(info, saltar_b(info, proxy));
    let raw = s(&["-f", "rawvideo", "-pix_fmt", "bgr24", "pipe:1"]);
    let fps = float_repr(per_second);
    match proxy {
        None => {
            // sin proxy, primero se eligen los cuadros y luego se escalan: los mismos cuadros (`fps` solo
            // mira las marcas de tiempo), pero `scale_cuda` pasa de ~30 a 4 cuadros por segundo de video.
            // Medido en un 4K 4:2:2 de Sony: la GPU ocupada solo por decodificar baja del 25 % al 13 %.
            let mut vf = vec![format!("fps={fps}")];
            vf.extend(chain.iter().cloned());
            vf.extend(["hwdownload".into(), "format=nv12".into(), "format=bgr24".into()]);
            cmd.extend(["-vf".to_string(), vf.join(",")]);
            cmd.extend(raw);
        }
        Some(p) => {
            let graph = format!("[0:v]{},split=2[a][b];[a]fps={fps},hwdownload,format=nv12,format=bgr24[raw];[b]hwdownload,format=nv12[pv]", chain.join(","));
            cmd.extend(["-filter_complex".to_string(), graph, "-map".into(), "[raw]".into()]);
            cmd.extend(raw);
            cmd.extend(["-map".to_string(), "[pv]".into()]);
            cmd.extend(s(PROXY_ARGS));
            cmd.push(p.to_string_lossy().to_string());
        }
    }
    Some(cmd)
}

/// Comando de CPU (respaldo), escalador "area" con el que se calibraron los umbrales.
pub fn cpu_cmd(info: &ClipInfo, per_second: f64, size: (u32, u32), proxy: Option<&Path>) -> Vec<String> {
    let (w, h) = size;
    let mut cmd = s(&["ffmpeg", "-v", "error", "-y", "-hwaccel", "auto"]);
    if saltar_b(info, proxy) {
        cmd.extend(s(&["-skip_frame", SALTAR_CUADROS]));
    }
    cmd.push("-i".into());
    cmd.push(info.source().to_string_lossy().to_string());
    let raw = s(&["-f", "rawvideo", "-pix_fmt", "bgr24", "pipe:1"]);
    let fps = float_repr(per_second);
    match proxy {
        None => {
            cmd.extend(["-vf".to_string(), format!("fps={fps},scale={w}:{h}:flags=area")]);
            cmd.extend(raw);
        }
        Some(p) => {
            let graph = format!("[0:v]scale={w}:{h}:flags=area,split=2[a][pv];[a]fps={fps}[raw]");
            cmd.extend(["-filter_complex".to_string(), graph, "-map".into(), "[raw]".into()]);
            cmd.extend(raw);
            cmd.extend(["-map".to_string(), "[pv]".into()]);
            cmd.extend(s(PROXY_ARGS));
            cmd.push(p.to_string_lossy().to_string());
        }
    }
    cmd
}

/// Comandos de ffmpeg (GPU primero, CPU de respaldo) que solo escriben el proxy de un clip analizado.
pub fn proxy_commands(info: &ClipInfo, proxy: &Path) -> Vec<Vec<String>> {
    let size = scaled_size(info);
    let mut cmds = Vec::new();
    if let Some(steps) = transpose(info.rotation) {
        let (w, h) = size;
        let pre = if steps.is_empty() { (w, h) } else { (h, w) };
        let mut chain = vec![format!("scale_cuda={}:{}:format=nv12:interp_algo=lanczos", pre.0, pre.1)];
        chain.extend(steps);
        chain.extend(["hwdownload".to_string(), "format=nv12".into()]);
        // el proxy necesita todos los cuadros
        let mut c = gpu_input(info, false);
        c.extend(["-vf".to_string(), chain.join(",")]);
        c.extend(s(PROXY_ARGS));
        c.push(proxy.to_string_lossy().to_string());
        cmds.push(c);
    }
    let mut c = s(&["ffmpeg", "-v", "error", "-y", "-hwaccel", "auto", "-i"]);
    c.push(info.source().to_string_lossy().to_string());
    c.extend(["-vf".to_string(), format!("scale={}:{}:flags=area", size.0, size.1)]);
    c.extend(s(PROXY_ARGS));
    c.push(proxy.to_string_lossy().to_string());
    cmds.push(c);
    cmds
}

/// Una muestra decodificada.
pub struct Sampled {
    /// Segundos desde el inicio del clip.
    pub t: f64,
    /// Cuadro a 960 px.
    pub frame: Frame,
    /// Copia de detalle (hasta 1920 px) solo en la pasada con zoom; `None` = la misma que `frame`.
    pub detail: Option<Frame>,
}

/// Qué pasó en la decodificación (`stats` de Python).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DecodeStats {
    /// Decodificado en la GPU.
    pub gpu: bool,
    /// Corrección del Laplaciano por el escalador (1/1.103 en GPU, 1 en CPU).
    pub lap_scale: f64,
    /// El archivo dejó de decodificarse antes del final (cortado o dañado): segundo de la última muestra.
    pub cut_at: Option<f64>,
}

/// Tiempo máximo sin recibir ni un cuadro de ffmpeg antes de darlo por colgado (archivo dañado que
/// deja al decodificador esperando). Muy por encima de lo que tarda el cuadro más lento de un 4K por CPU.
pub const DECODE_STALL: std::time::Duration = std::time::Duration::from_secs(180);

/// Lanza ffmpeg y entrega los cuadros por un canal desde un hilo lector (decodificar en paralelo con medir).
struct Decoder {
    child: Child,
    rx: mpsc::Receiver<Vec<u8>>,
    reader: Option<std::thread::JoinHandle<()>>,
    /// El lector llegó al final de la salida de ffmpeg (puede quedar algún cuadro en el canal).
    eof: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Lo que ffmpeg escribe en stderr (con `-v error`, solo errores), para explicar un fallo.
    errors: Option<std::thread::JoinHandle<String>>,
}

impl Decoder {
    fn spawn(cmd: &[String], frame_bytes: usize) -> Result<Decoder> {
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        no_window(&mut c);
        let mut child = c.spawn().with_context(|| format!("no se pudo ejecutar {}", cmd[0]))?;
        let mut out = child.stdout.take().ok_or_else(|| anyhow!("sin tubería de ffmpeg"))?;
        // pocos cuadros en vuelo: memoria acotada y ffmpeg no se adelanta demasiado
        let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(8);
        let eof = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let eof2 = eof.clone();
        let reader = std::thread::spawn(move || loop {
            let mut buf = vec![0u8; frame_bytes];
            if out.read_exact(&mut buf).is_err() {
                eof2.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }
            if tx.send(buf).is_err() {
                break;
            }
        });
        // stderr se vacía en otro hilo (si se llenara la tubería, ffmpeg se quedaría esperando); se
        // guardan solo los últimos 4 KB
        let errors = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut keep: Vec<u8> = Vec::new();
                let mut chunk = [0u8; 4096];
                while let Ok(n) = e.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    keep.extend_from_slice(&chunk[..n]);
                    if keep.len() > 4096 {
                        keep.drain(..keep.len() - 4096);
                    }
                }
                String::from_utf8_lossy(&keep).to_string()
            })
        });
        Ok(Decoder { child, rx, reader: Some(reader), eof, errors })
    }

    /// Espera a ffmpeg; verdadero si terminó bien, y sus mensajes de error.
    fn finish(mut self) -> (bool, String) {
        drop(std::mem::replace(&mut self.rx, mpsc::sync_channel(0).1));
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        let ok = self.child.wait().map(|s| s.success()).unwrap_or(false);
        let msg = self.errors.take().and_then(|h| h.join().ok()).unwrap_or_default();
        (ok, msg)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if self.reader.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// ffmpeg de un clip lanzado por adelantado ([`prepare`]): mientras se termina de analizar el clip
/// anterior, este ya arranca CUDA y NVDEC y decodifica sus primeras muestras. El resultado es el mismo.
pub struct Prepared {
    /// Comando del primer intento: solo se aprovecha si [`sample_with`] iba a lanzar exactamente ese.
    cmd: Vec<String>,
    dec: Decoder,
}

/// Lanza ya el primer intento de decodificación de un clip (ver [`Prepared`]).
pub fn prepare(info: &ClipInfo, per_second: f64, detail: bool, proxy: Option<&Path>) -> Result<Prepared> {
    let (w, h) = scaled_size(info);
    let (dw, dh) = if detail { detail_size(info) } else { (w, h) };
    let part = proxy.map(|p| p.with_extension("part.mp4"));
    if let Some(parent) = part.as_ref().and_then(|p| p.parent()) {
        std::fs::create_dir_all(parent)?;
    }
    let (cmd, _) = attempts(info, per_second, (dw, dh), part.as_deref()).into_iter().next().ok_or_else(|| anyhow!("sin comando de decodificación"))?;
    let dec = Decoder::spawn(&cmd, (dw * dh * 3) as usize)?;
    Ok(Prepared { cmd, dec })
}

/// Intentos de decodificación en orden (comando, ¿GPU?): GPU (NVDEC) y, si no da cuadros, CPU.
fn attempts(info: &ClipInfo, per_second: f64, (dw, dh): (u32, u32), part: Option<&Path>) -> Vec<(Vec<String>, bool)> {
    let mut attempts: Vec<(Vec<String>, bool)> = Vec::new();
    // CAMCUTS_DECODE=cpu fuerza la ruta de CPU (pruebas, o equipos donde NVDEC da problemas)
    let cpu_only = std::env::var("CAMCUTS_DECODE").map(|v| v.eq_ignore_ascii_case("cpu")).unwrap_or(false);
    if let Some(g) = gpu_cmd(info, per_second, (dw, dh), part).filter(|_| !cpu_only) {
        attempts.push((g, true));
    }
    let mut cpu = cpu_cmd(info, per_second, (dw, dh), part);
    if cpu_only {
        // sin `-hwaccel auto`, que en un equipo con NVIDIA también usaría su decodificador
        if let Some(i) = cpu.iter().position(|a| a == "-hwaccel") {
            cpu.drain(i..i + 2);
        }
    }
    attempts.push((cpu, false));
    attempts
}

/// Recorre las muestras de un clip a `per_second` por segundo, llamando a `f` con cada una.
///
/// Decodifica en la GPU cuando puede y si no por CPU. Con `proxy`, la misma decodificación escribe el
/// video de vista previa (primero como `.part.mp4`, renombrado al terminar bien). `f` devuelve `false`
/// para cortar (p. ej. al cancelar). La copia de detalle es de hasta 1920 px solo si `detail`.
pub fn sample(info: &ClipInfo, per_second: f64, detail: bool, proxy: Option<&Path>, f: impl FnMut(Sampled, &DecodeStats) -> Result<bool>) -> Result<DecodeStats> {
    sample_with(info, per_second, detail, proxy, None, &mut || {}, f)
}

/// [`sample`] aprovechando un ffmpeg ya lanzado con [`prepare`] (si es el mismo comando) y avisando con
/// `on_eof` en cuanto ffmpeg entrega su último cuadro: aún quedan muestras en vuelo por analizar, y ese
/// rato sirve para lanzar el ffmpeg del clip siguiente.
pub fn sample_with(
    info: &ClipInfo,
    per_second: f64,
    detail: bool,
    proxy: Option<&Path>,
    prepared: Option<Prepared>,
    on_eof: &mut dyn FnMut(),
    mut f: impl FnMut(Sampled, &DecodeStats) -> Result<bool>,
) -> Result<DecodeStats> {
    let (w, h) = scaled_size(info);
    let (dw, dh) = if detail { detail_size(info) } else { (w, h) };
    let part = proxy.map(|p| p.with_extension("part.mp4"));
    if let Some(p) = &part {
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let attempts = attempts(info, per_second, (dw, dh), part.as_deref());
    let mut ready = prepared.filter(|p| attempts.first().map(|a| a.0 == p.cmd).unwrap_or(false)).map(|p| p.dec);
    let frame_bytes = (dw * dh * 3) as usize;
    let mut stats = DecodeStats { gpu: false, lap_scale: 1.0, cut_at: None };
    let mut last_error = String::new();
    // Muestras ya entregadas a `f` por un intento anterior que ffmpeg terminó con error a mitad (p. ej. un
    // fallo de NVDEC): el intento siguiente (CPU) decodifica desde el principio y sigue desde ahí, sin
    // repetir las ya entregadas (el que llama ya las midió).
    let mut entregadas = 0usize;
    let n_attempts = attempts.len();
    for (k, (cmd, on_gpu)) in attempts.into_iter().enumerate() {
        let dec = match ready.take() {
            Some(d) => d,
            None => Decoder::spawn(&cmd, frame_bytes)?,
        };
        let mut i = 0usize;
        let mut stopped = false;
        let mut told = false;
        loop {
            // ffmpeg ya entregó todo (lo que queda son las muestras en vuelo del canal): se avisa una vez
            if !told && dec.eof.load(std::sync::atomic::Ordering::Relaxed) {
                told = true;
                on_eof();
            }
            let buf = match dec.rx.recv_timeout(DECODE_STALL) {
                Ok(b) => b,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // ffmpeg colgado: se mata (al soltar `dec`) y el clip se da por fallido
                    drop(dec);
                    if let Some(p) = &part {
                        let _ = std::fs::remove_file(p);
                    }
                    return Err(anyhow!("ffmpeg no entregó ningún cuadro en {} s (¿archivo dañado?)", DECODE_STALL.as_secs()));
                }
            };
            if i < entregadas {
                // ya la midió el intento anterior
                i += 1;
                continue;
            }
            if i == entregadas {
                // las muestras de aquí en adelante salen de este intento (escalador y corrección suyos)
                stats = DecodeStats { gpu: on_gpu, lap_scale: if on_gpu { GPU_LAP_SCALE } else { 1.0 }, cut_at: None };
            }
            let big = Frame { width: dw, height: dh, data: buf };
            let (frame, det) = if (dw, dh) == (w, h) {
                (big, None)
            } else {
                let small = crate::image::resize_area_u8(&big.data, dw as usize, dh as usize, 3, w as usize, h as usize);
                (Frame { width: w, height: h, data: small }, Some(big))
            };
            let go = f(Sampled { t: i as f64 / per_second, frame, detail: det }, &stats)?;
            i += 1;
            if !go {
                stopped = true;
                break;
            }
        }
        if stopped {
            drop(dec); // mata ffmpeg
            return Ok(stats);
        }
        if !told {
            on_eof();
        }
        let (ok, msg) = dec.finish();
        // contando las ya entregadas por un intento anterior (este puede haber dado menos)
        let n = i.max(entregadas);
        if n > 0 {
            // los cuadros se acabaron más de 1 s antes de la duración que dice el archivo: está cortado (se
            // analiza lo que hay). ffmpeg suele terminar bien aunque falte el final, así que no basta `ok`.
            let last_t = (n - 1) as f64 / per_second;
            let falta = last_t + 1.0 < info.duration;
            // ffmpeg terminó con error a mitad: puede ser la GPU y no el archivo. Antes de darlo por cortado
            // se prueba el intento siguiente (CPU), que sigue desde la última muestra entregada
            if !ok && falta && k + 1 < n_attempts {
                entregadas = n;
                last_error = msg;
                continue;
            }
            if falta {
                stats.cut_at = Some(last_t);
            }
            if let (Some(part), Some(proxy)) = (&part, proxy) {
                // solo un proxy completo: el de un archivo cortado (ffmpeg suele terminar bien) se acabaría
                // antes de tiempo y, como `proxy_vigente` lo acepta al reanalizar, ya no se rehacía
                if ok && stats.cut_at.is_none() && part.exists() {
                    let _ = std::fs::rename(part, proxy);
                } else {
                    let _ = std::fs::remove_file(part);
                }
            }
            return Ok(stats);
        }
        // esta fuente (o este equipo) no admite la GPU: se prueba la CPU
        last_error = msg;
    }
    if entregadas > 0 {
        // el último intento no dio nada nuevo: se queda lo entregado, como archivo cortado
        if let Some(p) = &part {
            let _ = std::fs::remove_file(p);
        }
        stats.cut_at = Some((entregadas - 1) as f64 / per_second);
        return Ok(stats);
    }
    if let Some(p) = &part {
        let _ = std::fs::remove_file(p);
    }
    if last_error.trim().is_empty() && info.duration < 0.5 / per_second {
        // el filtro `fps` redondea al más cercano: un clip de menos de media muestra no da ninguna
        return Err(anyhow!("clip demasiado corto ({:.2} s) para muestrearlo a {per_second}/s", info.duration));
    }
    Err(anyhow!("ffmpeg no pudo decodificar ningún cuadro ({})", short_error(if last_error.trim().is_empty() { "sin mensaje" } else { &last_error })))
}

/// Decodifica el audio de un clip como mono de 16 kHz en `f32` (vacío si no tiene audio).
pub fn audio_16k(path: &Path) -> Vec<f32> {
    match run_quiet("ffmpeg", &["-v", "error", "-i", &path.to_string_lossy(), "-vn", "-ac", "1", "-ar", "16000", "-f", "s16le", "pipe:1"]) {
        Ok(b) => b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La firma de x264 se encuentra entre bytes que no son texto (antes: pánico al cortar a mitad de un
    /// carácter, y el clip salía "fallo interno al abrir el clip").
    #[test]
    fn proxy_vigente_por_su_firma() {
        let dir = std::env::temp_dir().join(format!("camcuts_proxy_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let basura: Vec<u8> = (0..3000u32).map(|i| (i * 37 % 256) as u8).collect();
        let firma = |opc: &str| [basura.as_slice(), b"x264 - core 165 - options: ", opc.as_bytes(), basura.as_slice()].concat();
        let nuevo = dir.join("nuevo.mp4");
        std::fs::write(&nuevo, firma("cabac=1 deblock=1:0:0 rc=crf crf=20.0 qcomp=0.60")).unwrap();
        assert!(proxy_vigente(&nuevo));
        let viejo = dir.join("viejo.mp4");
        std::fs::write(&viejo, firma("cabac=0 deblock=0:0:0 rc=crf crf=28.0 qcomp=0.60")).unwrap();
        assert!(!proxy_vigente(&viejo));
        // la firma justo al final, con caracteres "partidos" alrededor
        let cola = dir.join("cola.mp4");
        std::fs::write(&cola, [&[0xE2u8, 0x82][..], b"x264 - core 165 deblock=1:0:0 crf=20.0", &[0xF0, 0x9F]].concat()).unwrap();
        assert!(proxy_vigente(&cola));
        assert!(!proxy_vigente(&dir.join("no_existe.mp4")));
        // proxy largo con `+faststart`: ftyp + moov de 600 KB + mdat con la firma más allá de los 512 KB
        let caja = |tipo: &[u8], carga: &[u8]| [&((carga.len() + 8) as u32).to_be_bytes()[..], tipo, carga].concat();
        let largo = dir.join("largo.mp4");
        let mdat = caja(b"mdat", &firma("cabac=1 deblock=1:0:0 rc=crf crf=20.0"));
        std::fs::write(&largo, [caja(b"ftyp", b"isom\0\0\x02\0"), caja(b"moov", &vec![7u8; 600 * 1024]), mdat].concat()).unwrap();
        assert!(proxy_vigente(&largo), "la firma al principio de mdat, tras un moov grande");
        // lo mismo con un proxy viejo: no vale
        let largo_viejo = dir.join("largo_viejo.mp4");
        let mdat = caja(b"mdat", &firma("cabac=0 deblock=0:0:0 rc=crf crf=28.0"));
        std::fs::write(&largo_viejo, [caja(b"ftyp", b"isom\0\0\x02\0"), caja(b"moov", &vec![7u8; 600 * 1024]), mdat].concat()).unwrap();
        assert!(!proxy_vigente(&largo_viejo));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El drop-frame sale también del XML incrustado en el MP4 (sin `M01.XML` al lado), como el color.
    #[test]
    fn drop_frame_del_xml_incrustado() {
        let dir = std::env::temp_dir().join(format!("camcuts_df_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let xml = |ltc: &str| format!(r#"<NonRealTimeMeta><LtcChangeTable><LtcChange frameCount="0" value="{ltc}" status="increment"/></LtcChangeTable><Item name="CaptureGammaEquation" value="s-log3-cine"/></NonRealTimeMeta>"#);
        let relleno = vec![0u8; 300 * 1024];
        // NDF (0x22: sin el bit 0x40)
        let ndf = dir.join("C0001.MP4");
        std::fs::write(&ndf, [relleno.as_slice(), xml("22334455").as_bytes(), &[0u8; 700]].concat()).unwrap();
        assert_eq!(sony_drop_frame(&ndf), Some(false));
        // DF (0x49: con el bit 0x40)
        let df = dir.join("C0002.MP4");
        std::fs::write(&df, [relleno.as_slice(), xml("49334455").as_bytes(), &[0u8; 700]].concat()).unwrap();
        assert_eq!(sony_drop_frame(&df), Some(true));
        // sin XML en ningún sitio
        let nada = dir.join("C0003.MP4");
        std::fs::write(&nada, &relleno).unwrap();
        assert_eq!(sony_drop_frame(&nada), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Las copias "._" de macOS no son clips, también sueltas.
    #[test]
    fn basura_de_macos() {
        assert!(es_basura_mac(Path::new("D:\\Boda\\._C0001.MP4")));
        assert!(!es_basura_mac(Path::new("D:\\Boda\\C0001.MP4")));
        assert!(!es_basura_mac(Path::new("D:\\._Boda\\C0001.MP4")));
    }

    fn info(tc: &str, num: i64, den: i64) -> ClipInfo {
        ClipInfo { path: "x.mp4".into(), width: 3840, height: 2160, fps: Fraction::new(num, den), duration: 1.0, decode_path: None, start_tc: tc.into(), rotation: 0, saltar_b: false }
    }

    #[test]
    fn timecode_drop_frame() {
        // 17:42:22;22 a 29.97 DF: el valor que Python escribió en el OTIO de bench_oct27 para C9168
        let f = start_frames(&info("17:42:22;22", 30000, 1001));
        assert_eq!(f, ((17 * 3600 + 42 * 60 + 22) * 30 + 22 - 2 * (1062 - 106)) as f64);
        assert_eq!(start_frames(&info("01:00:00:00", 25, 1)), 90000.0);
    }

    #[test]
    fn drop_frame_solo_en_ntsc_multiplo_de_2997() {
        // pista "rtmd" de Sony (sin marca de DF): a 29.97/59.94/119.88 se asume DF, a 23.976 nunca
        let data = |tc: &str| serde_json::json!({"streams": [{"codec_tag_string": "rtmd", "tags": {"timecode": tc}}], "format": {}});
        assert_eq!(start_timecode(&data("10:00:00:00"), Fraction::new(30000, 1001)), "10:00:00;00");
        assert_eq!(start_timecode(&data("10:00:00:00"), Fraction::new(60000, 1001)), "10:00:00;00");
        assert_eq!(start_timecode(&data("10:00:00:00"), Fraction::new(120000, 1001)), "10:00:00;00");
        assert_eq!(start_timecode(&data("10:00:00:00"), Fraction::new(24000, 1001)), "10:00:00:00");
        assert_eq!(start_timecode(&data("10:00:00:00"), Fraction::new(25, 1)), "10:00:00:00");
        // una caché antigua con ";" a 23.976 no debe restar cuadros
        assert_eq!(start_frames(&info("00:10:00;00", 24000, 1001)), (600 * 24) as f64);
    }

    #[test]
    fn gpu_sin_matriz_de_rotacion_solo_si_hay_rotacion() {
        let mut v = info("", 30000, 1001);
        let has = |c: &[String]| c.iter().position(|a| a == "-display_rotation").map(|i| i < c.iter().position(|a| a == "-i").unwrap());
        assert_eq!(has(&gpu_cmd(&v, 4.0, (960, 540), None).unwrap()), None);
        v.rotation = 90;
        assert_eq!(has(&gpu_cmd(&v, 4.0, (540, 960), Some(Path::new("p.mp4"))).unwrap()), Some(true));
        assert_eq!(has(&proxy_commands(&v, Path::new("p.mp4"))[0]), Some(true));
        // la CPU gira sola (autorrotación) y quita la matriz: no debe llevar la opción
        assert_eq!(has(&cpu_cmd(&v, 4.0, (540, 960), None)), None);
    }

    /// Sin proxy, `fps` va antes de `scale_cuda` (solo se escalan los cuadros que se analizan); con proxy
    /// se escala todo antes de repartir, porque el proxy lleva todos los cuadros.
    #[test]
    fn gpu_escala_solo_los_cuadros_muestreados() {
        let mut v = info("", 30000, 1001);
        let vf = |c: &[String], k: &str| c[c.iter().position(|a| a == k).unwrap() + 1].clone();
        let sin = vf(&gpu_cmd(&v, 4.0, (960, 540), None).unwrap(), "-vf");
        assert!(sin.starts_with("fps=4.0,scale_cuda=960:540"), "{sin}");
        v.rotation = 90;
        let girado = vf(&gpu_cmd(&v, 4.0, (540, 960), None).unwrap(), "-vf");
        assert!(girado.starts_with("fps=4.0,scale_cuda=960:540") && girado.contains("transpose_cuda"), "{girado}");
        let con = vf(&gpu_cmd(&v, 4.0, (540, 960), Some(Path::new("p.mp4"))).unwrap(), "-filter_complex");
        assert!(con.starts_with("[0:v]scale_cuda=") && con.contains("[a]fps=4.0,hwdownload"), "{con}");
    }

    #[test]
    fn tamanos() {
        assert_eq!(scaled_size(&info("", 25, 1)), (960, 540));
        let mut v = info("", 25, 1);
        v.width = 1080;
        v.height = 1920;
        assert_eq!(scaled_size(&v), (540, 960));
    }
}
