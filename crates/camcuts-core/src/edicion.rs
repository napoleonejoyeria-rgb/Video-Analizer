//! Edición de la app: joyas (por defecto) o bodas.
//!
//! El mismo código sirve a las dos ediciones. La edición se fija una sola vez al arrancar el binario
//! (`camcuts-bodas.exe` llama a [`fijar`] con [`Edicion::Bodas`]); si nadie la fija, se lee la variable
//! de entorno `CAMCUTS_EDICION` (`bodas`) y, si no está, es la de joyas. Así `camcuts.exe`, la CLI y
//! todas las pruebas de paridad con Python siguen exactamente como antes.
//!
//! Aquí vive también todo lo que separa los archivos de una edición de los de la otra, en un solo
//! sitio: la carpeta de resultados por defecto, el nombre de la app en los proyectos, la marca
//! `edition` de las cachés y los nombres de los archivos exportados. Así una sesión de bodas nunca
//! reutiliza ni pisa lo que hizo la de joyas (ni al revés).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Qué edición de CAMCUTS está corriendo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edicion {
    /// Selects de joyería: la joya nítida (comportamiento original, paridad con Python).
    Joyas,
    /// Selects de bodas: caras nítidas, momento de la boda, cámara estable.
    Bodas,
}

/// Nombres de las carpetas de resultados de todas las ediciones (al recorrer el material se saltan:
/// son resultados de CAMCUTS, no material).
pub const CARPETAS_RESULTADOS: [&str; 2] = ["CAMCUTS", "CAMCUTS Bodas"];

impl Edicion {
    /// Nombre de la app de esta edición: el campo `app` de los proyectos (`perfil.APP_NAME` de Python).
    pub fn nombre_app(self) -> &'static str {
        match self {
            Edicion::Joyas => "CAMCUTS",
            Edicion::Bodas => "CAMCUTS Bodas",
        }
    }

    /// Nombre de la carpeta de resultados por defecto, junto al material. Joyas conserva `CAMCUTS`
    /// (la de Python); bodas usa la suya para no mezclar cachés, proyectos ni exportaciones.
    pub fn carpeta(self) -> &'static str {
        match self {
            Edicion::Joyas => CARPETAS_RESULTADOS[0],
            Edicion::Bodas => CARPETAS_RESULTADOS[1],
        }
    }

    /// Carpeta de resultados por defecto de un material: `<material>\CAMCUTS` o `<material>\CAMCUTS Bodas`.
    pub fn carpeta_resultados(self, material: &Path) -> PathBuf {
        material.join(self.carpeta())
    }

    /// ¿Sirve para esta edición una caché con esta marca `edition`?
    ///
    /// El análisis de bodas escribe `"edition": "bodas"` (también el de Python); el de joyas no escribe
    /// nada, así que las cachés de joyas antiguas, las de Python incluidas, siguen valiendo para joyas.
    /// Una caché de la otra edición (o sin marca, en bodas) cuenta como vieja y se vuelve a analizar.
    pub fn acepta_cache(self, edition: Option<&str>) -> bool {
        let e = edition.map(str::trim).filter(|e| !e.is_empty());
        match self {
            Edicion::Joyas => e.map_or(true, |e| e.eq_ignore_ascii_case("joyas")),
            Edicion::Bodas => e.map_or(false, |e| e.eq_ignore_ascii_case("bodas")),
        }
    }

    /// Edición de un proyecto por su campo `app`. Sin campo es de joyas (como los de Python antiguos).
    pub fn de_app(app: Option<&str>) -> Edicion {
        match app {
            Some(a) if a.to_lowercase().contains("bodas") => Edicion::Bodas,
            _ => Edicion::Joyas,
        }
    }

    /// Prefijo de los archivos que exporta la ventana: `CAMCUTS_<criterio>_<modo>.otio` en joyas (como
    /// Python) y `CAMCUTS_Bodas_<criterio>_<modo>.otio` en bodas, para que nunca se pisen.
    pub fn prefijo_export(self) -> &'static str {
        match self {
            Edicion::Joyas => "CAMCUTS",
            Edicion::Bodas => "CAMCUTS_Bodas",
        }
    }

    /// Nombre (sin extensión) de los selects de la línea de comandos para un criterio: `selects_joyas`,
    /// `selects_social`… en joyas (como Python); en bodas `selects_bodas`, `selects_bodas_social`…
    pub fn nombre_selects(self, preset: &str) -> String {
        match self {
            Edicion::Joyas => format!("selects_{preset}"),
            Edicion::Bodas if preset == "bodas" => "selects_bodas".to_owned(),
            Edicion::Bodas => format!("selects_bodas_{preset}"),
        }
    }
}

static EDICION: OnceLock<Edicion> = OnceLock::new();

/// Fija la edición del proceso. Solo vale la primera vez; devuelve `true` si queda la pedida.
pub fn fijar(e: Edicion) -> bool {
    let _ = EDICION.set(e);
    actual() == e
}

/// Edición del proceso (la fijada, o la de `CAMCUTS_EDICION`, o joyas).
pub fn actual() -> Edicion {
    *EDICION.get_or_init(|| match std::env::var("CAMCUTS_EDICION") {
        Ok(v) if v.trim().eq_ignore_ascii_case("bodas") => Edicion::Bodas,
        _ => Edicion::Joyas,
    })
}

/// ¿Es la edición de bodas?
pub fn es_bodas() -> bool {
    actual() == Edicion::Bodas
}

/// Carpeta de resultados por defecto de un material para la edición del proceso.
pub fn carpeta_resultados(material: &Path) -> PathBuf {
    actual().carpeta_resultados(material)
}

/// ¿La caché de un clip es de la edición del proceso? (si no, cuenta como vieja: se vuelve a analizar)
pub fn cache_vigente(entry: &crate::project::ClipEntry) -> bool {
    actual().acepta_cache(entry.extra.get("edition").and_then(serde_json::Value::as_str))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Cada edición tiene su carpeta de resultados; joyas conserva la de Python.
    #[test]
    fn carpetas_separadas() {
        let m = Path::new("D:\\Boda");
        assert_eq!(Edicion::Joyas.carpeta_resultados(m), Path::new("D:\\Boda\\CAMCUTS"));
        assert_eq!(Edicion::Bodas.carpeta_resultados(m), Path::new("D:\\Boda\\CAMCUTS Bodas"));
        assert_ne!(Edicion::Joyas.prefijo_export(), Edicion::Bodas.prefijo_export());
        // ningún criterio de una edición da el mismo nombre que uno de la otra
        let joyas: Vec<String> = ["joyas", "social", "todo"].iter().map(|p| Edicion::Joyas.nombre_selects(p)).collect();
        let bodas: Vec<String> = ["bodas", "social", "todo"].iter().map(|p| Edicion::Bodas.nombre_selects(p)).collect();
        assert!(joyas.iter().all(|j| !bodas.contains(j)), "{joyas:?} {bodas:?}");
        assert_eq!(Edicion::Joyas.nombre_selects("joyas"), "selects_joyas");
        assert_eq!(Edicion::Bodas.nombre_selects("bodas"), "selects_bodas");
    }

    /// Las cachés de joyas sin marca siguen valiendo para joyas; las de la otra edición no.
    #[test]
    fn cache_de_otra_edicion_es_vieja() {
        assert!(Edicion::Joyas.acepta_cache(None));
        assert!(Edicion::Joyas.acepta_cache(Some("joyas")));
        assert!(!Edicion::Joyas.acepta_cache(Some("bodas")));
        assert!(Edicion::Bodas.acepta_cache(Some("bodas")));
        assert!(!Edicion::Bodas.acepta_cache(None));
        assert!(!Edicion::Bodas.acepta_cache(Some("")));
        assert!(!Edicion::Bodas.acepta_cache(Some("joyas")));
    }

    /// El campo `app` de los proyectos dice su edición (sin campo: joyas).
    #[test]
    fn edicion_del_proyecto() {
        assert_eq!(Edicion::de_app(None), Edicion::Joyas);
        assert_eq!(Edicion::de_app(Some("CAMCUTS")), Edicion::Joyas);
        assert_eq!(Edicion::de_app(Some("CAMCUTS Bodas")), Edicion::Bodas);
        assert_eq!(Edicion::de_app(Some(Edicion::Bodas.nombre_app())), Edicion::Bodas);
    }
}
