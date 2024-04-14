use std::sync::Arc;

use axum::http::header;
use axum::response::{IntoResponse, Response};
use image::{GenericImage, ImageBuffer, ImageFormat, Rgba};

use crate::api;

#[derive(Clone)]
pub struct Image {
    pub data: Box<[u8]>,
    pub mime_type: Arc<str>,
}

impl Image {
    pub fn new(data: Box<[u8]>, mime_type: Arc<str>) -> Self {
        Self { data, mime_type }
    }
}

impl IntoResponse for Image {
    fn into_response(self) -> Response {
        (
            [(header::CONTENT_TYPE, self.mime_type.to_string())],
            self.data,
        )
            .into_response()
    }
}

pub async fn make_preview(posts: api::Posts) -> Option<Image> {
    log::info!("generating preview...");

    let urls = posts
        .iter()
        .map(|post| post.preview.url.clone())
        .map(api::get_image);

    let previews = futures::future::try_join_all(urls).await.ok()?;

    let preview = tokio::task::spawn_blocking(move || {
        let mut pic: ImageBuffer<Rgba<u8>, _> = ImageBuffer::new(1500, 1500);

        for (image, i) in previews.into_iter().zip(0_u32..) {
            let mem = image::load_from_memory(&image.data).ok()?;

            let x = (i % 10) * 150 + (150 - mem.width()) / 2;
            let y = (i / 10) * 150 + (150 - mem.height()) / 2;

            pic.copy_from(&mem, x, y).ok()?;
        }

        // todo: benchmark this
        let mut buf = std::io::Cursor::new(Vec::new());
        pic.write_to(&mut buf, ImageFormat::Png).ok()?;

        Some(Image::new(
            buf.into_inner().into_boxed_slice(),
            "image/png".into(),
        ))
    })
    .await;

    log::info!("finished generating preview");

    preview.ok().flatten()
}
