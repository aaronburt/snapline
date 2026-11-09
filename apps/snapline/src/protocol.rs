use snapline_services::thumbs::{self, Thumbnail};
use snapline_services::Services;
use tauri::http::header::{HeaderValue, CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use tauri::http::{Request, Response, StatusCode};

const IMAGE_CACHE: &str = "public, max-age=86400";
const NOT_FOUND_CACHE: &str = "no-store";
const NOT_FOUND_BODY: &[u8] = b"Not found";

pub fn handle(services: &Services, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let Some(id) = thumbs::parse_id(request.uri().path()) else {
        return not_found();
    };
    match services.thumbs().get(id) {
        Ok(thumbnail) => image(thumbnail),
        Err(_) => not_found(),
    }
}

fn image(thumbnail: Thumbnail) -> Response<Vec<u8>> {
    let mut response = Response::new(thumbnail.bytes);
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(thumbnail.mime));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(IMAGE_CACHE));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

fn not_found() -> Response<Vec<u8>> {
    let mut response = Response::new(NOT_FOUND_BODY.to_vec());
    *response.status_mut() = StatusCode::NOT_FOUND;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(NOT_FOUND_CACHE));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &str) -> Request<Vec<u8>> {
        let uri = format!("http://thumb.localhost{path}");
        Request::builder()
            .uri(uri.as_str())
            .body(Vec::new())
            .expect("request")
    }

    #[test]
    fn malformed_paths_are_not_found() {
        let services = Services::new();
        for path in [
            "/",
            "",
            "/abc",
            "/-1",
            "/0",
            "/1.5",
            "/1/2",
            "//1",
            "/../1",
            "/9223372036854775808",
        ] {
            let response = handle(&services, request(path));
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[test]
    fn a_valid_id_without_a_history_is_not_found() {
        let services = Services::new();
        let response = handle(&services, request("/1"));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.body(), NOT_FOUND_BODY);
    }

    #[test]
    fn a_served_image_has_a_type_and_caching_headers() {
        let response = image(Thumbnail {
            bytes: vec![1, 2, 3],
            mime: "image/webp",
        });
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), &vec![1, 2, 3]);
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .map(HeaderValue::as_bytes),
            Some(b"image/webp".as_slice())
        );
        assert_eq!(
            response
                .headers()
                .get(CACHE_CONTROL)
                .map(HeaderValue::as_bytes),
            Some(IMAGE_CACHE.as_bytes())
        );
        assert_eq!(
            response
                .headers()
                .get(X_CONTENT_TYPE_OPTIONS)
                .map(HeaderValue::as_bytes),
            Some(b"nosniff".as_slice())
        );
    }

    #[test]
    fn a_not_found_response_is_uncached_and_reveals_nothing() {
        let response = not_found();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response
                .headers()
                .get(CACHE_CONTROL)
                .map(HeaderValue::as_bytes),
            Some(NOT_FOUND_CACHE.as_bytes())
        );
        let body = String::from_utf8_lossy(response.body());
        assert!(!body.contains("SnapLine"));
        assert!(!body.contains('/'));
        assert!(!body.contains('\\'));
    }
}
