#![doc = "Uploaders, HTTP and the upload queue for Snapline. This crate owns the Uploader trait, Imgur, S3 SigV4, custom ShareX uploaders, Gist, SFTP, EXIF stripping, retry decisions and link formats. It must not depend on Win32, the Tauri UI or unsafe code."]
#![forbid(unsafe_code)]

pub mod exif;
