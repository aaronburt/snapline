use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
}

impl ImageFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
        }
    }

    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::WebP => "image/webp",
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ImageError {
    #[error("The image is empty.")]
    Empty,
    #[error("The image stride is too small.")]
    Stride,
    #[error("The image buffer is too short.")]
    ShortBuffer,
    #[error("The pixel is outside the image.")]
    Outside,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    stride: usize,
    pixels: Vec<u8>,
}

impl Image {
    pub fn new(
        width: u32,
        height: u32,
        stride: usize,
        pixels: Vec<u8>,
    ) -> Result<Self, ImageError> {
        if width == 0 || height == 0 {
            return Err(ImageError::Empty);
        }
        let row = usize::try_from(width).map_err(|_| ImageError::Stride)? * 4;
        if stride < row {
            return Err(ImageError::Stride);
        }
        let expected = stride
            .checked_mul(usize::try_from(height).map_err(|_| ImageError::Stride)?)
            .ok_or(ImageError::ShortBuffer)?;
        if pixels.len() < expected {
            return Err(ImageError::ShortBuffer);
        }
        Ok(Self {
            width,
            height,
            stride,
            pixels,
        })
    }

    pub fn bgra(width: u32, height: u32) -> Result<Self, ImageError> {
        let stride = usize::try_from(width)
            .map_err(|_| ImageError::Stride)?
            .checked_mul(4)
            .ok_or(ImageError::Stride)?;
        let length = stride
            .checked_mul(usize::try_from(height).map_err(|_| ImageError::Stride)?)
            .ok_or(ImageError::ShortBuffer)?;
        Self::new(width, height, stride, vec![0; length])
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub const fn stride(&self) -> usize {
        self.stride
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    pub fn pixel(&self, x: u32, y: u32) -> Result<[u8; 4], ImageError> {
        if x >= self.width || y >= self.height {
            return Err(ImageError::Outside);
        }
        let offset = y as usize * self.stride + x as usize * 4;
        let pixel = &self.pixels[offset..offset + 4];
        Ok([pixel[0], pixel[1], pixel[2], pixel[3]])
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, value: [u8; 4]) -> Result<(), ImageError> {
        if x >= self.width || y >= self.height {
            return Err(ImageError::Outside);
        }
        let offset = y as usize * self.stride + x as usize * 4;
        self.pixels[offset..offset + 4].copy_from_slice(&value);
        Ok(())
    }

    pub fn row(&self, y: u32) -> Option<&[u8]> {
        if y >= self.height {
            return None;
        }
        let start = y as usize * self.stride;
        self.pixels.get(start..start + self.width as usize * 4)
    }

    pub fn row_mut(&mut self, y: u32) -> Option<&mut [u8]> {
        if y >= self.height {
            return None;
        }
        let start = y as usize * self.stride;
        let end = start + self.width as usize * 4;
        self.pixels.get_mut(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_image_has_bgra_rows() {
        let image = Image::bgra(3, 2).expect("image");
        assert_eq!(image.width(), 3);
        assert_eq!(image.height(), 2);
        assert_eq!(image.stride(), 12);
        assert_eq!(image.pixels().len(), 24);
        assert_eq!(image.pixel(0, 0), Ok([0, 0, 0, 0]));
    }

    #[test]
    fn pixels_round_trip() {
        let mut image = Image::bgra(2, 2).expect("image");
        image.set_pixel(1, 1, [1, 2, 3, 255]).expect("pixel");
        assert_eq!(image.pixel(1, 1), Ok([1, 2, 3, 255]));
    }

    #[test]
    fn stride_is_respected() {
        let mut pixels = vec![0_u8; 16];
        pixels[8] = 9;
        let image = Image::new(1, 2, 8, pixels).expect("image");
        assert_eq!(image.pixel(0, 1), Ok([9, 0, 0, 0]));
    }

    #[test]
    fn rejects_invalid_images() {
        assert_eq!(Image::new(0, 1, 0, Vec::new()), Err(ImageError::Empty));
        assert_eq!(Image::new(1, 1, 3, vec![0; 12]), Err(ImageError::Stride));
        assert_eq!(
            Image::new(1, 1, 4, vec![0; 3]),
            Err(ImageError::ShortBuffer)
        );
    }

    #[test]
    fn rejects_pixels_outside() {
        let mut image = Image::bgra(1, 1).expect("image");
        assert_eq!(image.pixel(1, 0), Err(ImageError::Outside));
        assert_eq!(image.set_pixel(0, 1, [0; 4]), Err(ImageError::Outside));
    }
}
