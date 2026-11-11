use std::sync::Arc;

use crate::geometry::{Point, Px, Rect, Size};
use crate::image::{Image, ImageError};

use super::command::{Command, CommandError, History};
use super::raster::{crop_image, Raster};
use super::shape::{Annotation, Shape, ShapeId, Style};

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    image: Arc<Image>,
    crop: Rect,
    annotations: Vec<Annotation>,
    next_id: u64,
    history: History,
}

impl Document {
    pub fn new(image: Image) -> Self {
        Self::shared(Arc::new(image))
    }

    pub fn shared(image: Arc<Image>) -> Self {
        let crop = Rect::new(
            Px::ZERO,
            Px::ZERO,
            Px(image.width() as i32),
            Px(image.height() as i32),
        );
        Self {
            image,
            crop,
            annotations: Vec::new(),
            next_id: 1,
            history: History::new(),
        }
    }

    pub fn image(&self) -> &Image {
        &self.image
    }

    pub const fn crop(&self) -> Rect {
        self.crop
    }

    pub fn full_rect(&self) -> Rect {
        Rect::new(
            Px::ZERO,
            Px::ZERO,
            Px(self.image.width() as i32),
            Px(self.image.height() as i32),
        )
    }

    pub const fn size(&self) -> Size {
        Size::new(self.crop.width, self.crop.height)
    }

    pub fn annotations(&self) -> &[Annotation] {
        &self.annotations
    }

    pub fn annotation(&self, id: ShapeId) -> Option<&Annotation> {
        self.annotations.iter().find(|item| item.id == id)
    }

    pub fn index_of(&self, id: ShapeId) -> Option<usize> {
        self.annotations.iter().position(|item| item.id == id)
    }

    pub fn len(&self) -> usize {
        self.annotations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }

    pub fn next_step(&self) -> u32 {
        let steps = self
            .annotations
            .iter()
            .filter(|item| matches!(item.shape, Shape::Step { .. }))
            .count();
        steps as u32 + 1
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo_len(&self) -> usize {
        self.history.undo_len()
    }

    pub fn redo_len(&self) -> usize {
        self.history.redo_len()
    }

    pub fn execute(&mut self, command: Command) -> Result<(), CommandError> {
        let mut history = std::mem::take(&mut self.history);
        let result = history.execute(self, command);
        self.history = history;
        result
    }

    pub fn undo(&mut self) -> Result<bool, CommandError> {
        let mut history = std::mem::take(&mut self.history);
        let result = history.undo(self);
        self.history = history;
        result
    }

    pub fn redo(&mut self) -> Result<bool, CommandError> {
        let mut history = std::mem::take(&mut self.history);
        let result = history.redo(self);
        self.history = history;
        result
    }

    pub fn add(&mut self, shape: Shape) -> Result<ShapeId, CommandError> {
        let id = ShapeId(self.next_id);
        self.next_id += 1;
        let index = self.annotations.len();
        self.execute(Command::Add {
            annotation: Annotation::new(id, shape),
            index,
        })?;
        Ok(id)
    }

    pub fn remove(&mut self, id: ShapeId) -> Result<(), CommandError> {
        let command = Command::remove(self, id).ok_or(CommandError::Missing)?;
        self.execute(command)
    }

    pub fn move_by(&mut self, id: ShapeId, delta: Point) -> Result<(), CommandError> {
        let command = Command::move_by(self, id, delta).ok_or(CommandError::Missing)?;
        self.execute(command)
    }

    pub fn resize(&mut self, id: ShapeId, rect: Rect) -> Result<(), CommandError> {
        let command = Command::resize(self, id, rect).ok_or_else(|| {
            if self.index_of(id).is_some() {
                CommandError::EmptyBounds
            } else {
                CommandError::Missing
            }
        })?;
        self.execute(command)
    }

    pub fn restyle(&mut self, id: ShapeId, style: Style) -> Result<(), CommandError> {
        let command = Command::restyle(self, id, style).ok_or(CommandError::Missing)?;
        self.execute(command)
    }

    pub fn edit_text(&mut self, id: ShapeId, text: impl Into<String>) -> Result<(), CommandError> {
        let command = Command::edit_text(self, id, text).ok_or(CommandError::Missing)?;
        self.execute(command)
    }

    pub fn crop_to(&mut self, rect: Rect) -> Result<(), CommandError> {
        let command = Command::crop(self, rect)?;
        self.execute(command)
    }

    pub fn render(&self) -> Result<Image, ImageError> {
        let full = self.render_full()?;
        crop_image(&full, self.crop)
    }

    pub fn render_full(&self) -> Result<Image, ImageError> {
        let mut raster = Raster::from_image(Image::clone(&self.image));
        for annotation in &self.annotations {
            raster.draw(&annotation.shape, &self.image);
        }
        Ok(raster.into_image())
    }

    pub(crate) fn insert(&mut self, index: usize, annotation: Annotation) {
        let index = index.min(self.annotations.len());
        self.annotations.insert(index, annotation);
        self.renumber_steps();
    }

    pub(crate) fn take(&mut self, id: ShapeId) -> Option<(usize, Annotation)> {
        let index = self.index_of(id)?;
        let annotation = self.annotations.remove(index);
        self.renumber_steps();
        Some((index, annotation))
    }

    pub(crate) fn replace(&mut self, annotation: Annotation) -> bool {
        let Some(existing) = self
            .annotations
            .iter_mut()
            .find(|item| item.id == annotation.id)
        else {
            return false;
        };
        *existing = annotation;
        true
    }

    pub(crate) fn set_crop(&mut self, rect: Rect) {
        self.crop = rect;
    }

    fn renumber_steps(&mut self) {
        let mut number = 1;
        for annotation in &mut self.annotations {
            if let Shape::Step { number: value, .. } = &mut annotation.shape {
                *value = number;
                number += 1;
            }
        }
    }
}
