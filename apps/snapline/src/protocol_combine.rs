use snapline_services::tools::combine::{self, Preview};
use tauri::http::header::{HeaderValue, CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use tauri::http::{Request, Response, StatusCode};

const IMAGE_CACHE: &str = "no-store";
const NOT_FOUND_CACHE: &str = "no-store";
const NOT_FOUND_BODY: &[u8] = b"Not found";

pub fn handle(request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let Some(id) = combine::parse_id(request.uri().path()) else {
        return not_found();
    };
    match combine::preview(id) {
        Some(preview) => image(preview),
        None => not_found(),
    }
}

fn image(preview: Preview) -> Response<Vec<u8>> {
    let mut response = Response::new(preview.bytes);
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(preview.mime));
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
        let uri = format!("http://combine.localhost{path}");
        Request::builder()
            .uri(uri.as_str())
            .body(Vec::new())
            .expect("request")
    }

    #[test]
    fn malformed_paths_are_not_found() {
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
            "/%31",
            "/4294967296",
        ] {
            let response = handle(request(path));
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[test]
    fn a_missing_job_is_not_found() {
        let response = handle(request("/1"));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.body(), NOT_FOUND_BODY);
    }

    #[test]
    fn a_served_preview_has_the_image_type_and_is_not_cached() {
        let response = image(Preview {
            bytes: vec![1, 2, 3],
            mime: "image/png",
        });
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body(), &vec![1, 2, 3]);
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .map(HeaderValue::as_bytes),
            Some(b"image/png".as_slice())
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
    fn a_webp_preview_keeps_its_type() {
        let response = image(Preview {
            bytes: vec![9],
            mime: "image/webp",
        });
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .map(HeaderValue::as_bytes),
            Some(b"image/webp".as_slice())
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
        assert!(!body.contains("Snapline"));
        assert!(!body.contains('/'));
        assert!(!body.contains('\\'));
    }
}
