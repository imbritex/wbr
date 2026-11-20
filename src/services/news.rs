pub struct NewsItem {
    pub title: String,
    pub source: String,
    pub summary: String,
}

pub struct NewsService;

impl NewsService {
    pub fn new() -> Self {
        Self
    }

    pub fn placeholder_items(&self) -> Vec<NewsItem> {
        vec![NewsItem {
            title: "wb-r inicializado con Slint + Material 3".into(),
            source: "Sistema".into(),
            summary: "Base nativa en Rust lista para conectar GameBanana, GitHub Releases y gestión de instancias.".into(),
        }]
    }
}
