use super::*;

fn encoded(image: &RgbaImage, format: ImageFormat) -> String {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image.clone())
        .to_rgb8()
        .write_to(&mut bytes, format)
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
}

#[test]
fn tray_icon_normalization_keeps_aspect_ratio_and_transparent_padding() {
    let image = RgbaImage::from_pixel(100, 50, image::Rgba([240, 10, 20, 255]));
    for (format, mime) in [
        (ImageFormat::Png, "image/png"),
        (ImageFormat::Jpeg, "image/jpeg"),
        (ImageFormat::WebP, "image/webp"),
    ] {
        let bytes = normalize(&encoded(&image, format), mime).unwrap();
        let normalized = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(normalized.dimensions(), (STORE_SIZE, STORE_SIZE));
        assert_eq!(normalized.get_pixel(STORE_SIZE / 2, 0).0[3], 0);
        assert_eq!(
            normalized.get_pixel(STORE_SIZE / 2, STORE_SIZE / 2).0[3],
            255
        );
        let tray = rasterize(&bytes, TRAY_SIZE).unwrap();
        assert_eq!(tray.dimensions(), (TRAY_SIZE, TRAY_SIZE));
        assert_eq!(tray.get_pixel(TRAY_SIZE / 2, 0).0[3], 0);
        assert!(tray_image(&bytes).is_ok());
        assert!(app_icon_image(&bytes).is_ok());
    }
}

#[test]
fn tray_icon_rejects_bad_mime_corruption_and_size_limits() {
    let image = RgbaImage::new(4, 4);
    let png = encoded(&image, ImageFormat::Png);
    assert!(normalize(&png, "image/jpeg")
        .unwrap_err()
        .contains("tray_icon_format"));
    assert!(normalize("invalid", "image/png").is_err());
    assert!(normalize("", "image/png").is_err());
    assert!(
        normalize(&"A".repeat(MAX_BYTES.div_ceil(3) * 4 + 1), "image/png")
            .unwrap_err()
            .contains("tray_icon_size")
    );
    let large = encoded(&RgbaImage::new(4097, 1), ImageFormat::Png);
    assert!(normalize(&large, "image/png").is_err());
}

#[test]
fn tray_icon_preserves_source_alpha() {
    let image = RgbaImage::from_pixel(64, 64, image::Rgba([10, 20, 30, 100]));
    let mut bytes = Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png).unwrap();
    let data = base64::engine::general_purpose::STANDARD.encode(bytes.into_inner());
    let normalized = normalize(&data, "image/png").unwrap();
    assert_eq!(
        image::load_from_memory(&normalized)
            .unwrap()
            .to_rgba8()
            .get_pixel(STORE_SIZE / 2, STORE_SIZE / 2)
            .0[3],
        100
    );
}

#[test]
fn tray_icon_rejects_animation_and_non_normalized_stored_images() {
    // Valid 1x1 APNG with one animation frame, generated from PNG chunks.
    let apng = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACGFjVEwAAAABAAAAALQt6aAAAAAaZmNUTAAAAAAAAAABAAAAAQAAAAAAAAAAAAEACgAAWn8w0AAAAA1JREFUeJxj+M/A8B8ABQAB/4mZPR0AAAAASUVORK5CYII=";
    assert!(normalize(apng, "image/png")
        .unwrap_err()
        .contains("animation"));
    let data = encoded(&RgbaImage::new(1, 1), ImageFormat::Png);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    assert_eq!(
        rasterize(&bytes, TRAY_SIZE).unwrap().dimensions(),
        (TRAY_SIZE, TRAY_SIZE)
    );
}

#[test]
fn legacy_64_tray_image_can_be_upscaled_for_the_app_icon() {
    let image = RgbaImage::from_pixel(TRAY_SIZE, TRAY_SIZE, image::Rgba([10, 20, 30, 200]));
    let bytes = encode_png(&image).unwrap();
    let tray = rasterize(&bytes, TRAY_SIZE).unwrap();
    assert_eq!(tray.dimensions(), (TRAY_SIZE, TRAY_SIZE));
    assert_eq!(tray.get_pixel(0, 0).0, [10, 20, 30, 200]);
    let app = rasterize(&bytes, STORE_SIZE).unwrap();
    assert_eq!(app.dimensions(), (STORE_SIZE, STORE_SIZE));
    assert_eq!(app.get_pixel(STORE_SIZE / 2, STORE_SIZE / 2).0[3], 200);
}

#[test]
fn jpeg_exif_orientation_is_applied_before_square_padding() {
    let source = RgbaImage::from_fn(2, 1, |x, _| {
        if x == 0 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        }
    });
    let mut jpeg = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(source)
        .to_rgb8()
        .write_to(&mut jpeg, ImageFormat::Jpeg)
        .unwrap();
    let jpeg = with_exif_orientation(jpeg.into_inner(), 6);
    let bytes = normalize(
        &base64::engine::general_purpose::STANDARD.encode(jpeg),
        "image/jpeg",
    )
    .unwrap();
    let normalized = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(normalized.dimensions(), (STORE_SIZE, STORE_SIZE));
    // Orientation 6 rotates 2x1 to 1x2, so padding is on the left and right.
    assert_eq!(normalized.get_pixel(0, STORE_SIZE / 2).0[3], 0);
    assert_eq!(normalized.get_pixel(STORE_SIZE / 2, 0).0[3], 255);
}

fn with_exif_orientation(jpeg: Vec<u8>, orientation: u16) -> Vec<u8> {
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG SOI");
    let mut tiff = Vec::from(b"Exif\0\0".as_slice());
    tiff.extend_from_slice(&[b'I', b'I', 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00]);
    tiff.extend_from_slice(&[0x01, 0x00]);
    tiff.extend_from_slice(&[0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00]);
    tiff.extend_from_slice(&orientation.to_le_bytes());
    tiff.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    let length = u16::try_from(tiff.len() + 2).unwrap();
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&tiff);
    out.extend_from_slice(&jpeg[2..]);
    out
}
