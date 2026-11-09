use thiserror::Error;

use crate::geometry::{Point, Rect};

use super::document::Document;
use super::shape::{Annotation, ShapeId, Style};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CommandError {
    #[error("The annotation no longer exists.")]
    Missing,
    #[error("The annotation bounds are empty.")]
    EmptyBounds,
    #[error("The crop rectangle is outside the image.")]
    EmptyCrop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Add {
        annotation: Annotation,
        index: usize,
    },
    Remove {
        annotation: Annotation,
        index: usize,
    },
    Move {
        before: Annotation,
        after: Annotation,
    },
    Resize {
        before: Annotation,
        after: Annotation,
    },
    Restyle {
        before: Annotation,
        after: Annotation,
    },
    Crop {
        before: Rect,
        after: Rect,
    },
    EditText {
        before: Annotation,
        after: Annotation,
    },
}

impl Command {
    pub const fn add(annotation: Annotation, index: usize) -> Self {
        Self::Add { annotation, index }
    }

    pub fn remove(document: &Document, id: ShapeId) -> Option<Self> {
        let index = document.index_of(id)?;
        let annotation = document.annotation(id)?.clone();
        Some(Self::Remove { annotation, index })
    }

    pub fn move_by(document: &Document, id: ShapeId, delta: Point) -> Option<Self> {
        let before = document.annotation(id)?.clone();
        let after = before
            .clone()
            .with_shape(before.shape.clone().translated(delta));
        Some(Self::Move { before, after })
    }

    pub fn resize(document: &Document, id: ShapeId, rect: Rect) -> Option<Self> {
        let before = document.annotation(id)?.clone();
        let from = before.shape.bounds();
        if from.is_empty() || rect.is_empty() {
            return None;
        }
        let after = before
            .clone()
            .with_shape(before.shape.clone().resized(from, rect));
        Some(Self::Resize { before, after })
    }

    pub fn restyle(document: &Document, id: ShapeId, style: Style) -> Option<Self> {
        let before = document.annotation(id)?.clone();
        let after = before
            .clone()
            .with_shape(before.shape.clone().with_style(style));
        Some(Self::Restyle { before, after })
    }

    pub fn edit_text(document: &Document, id: ShapeId, text: impl Into<String>) -> Option<Self> {
        let before = document.annotation(id)?.clone();
        let shape = before.shape.clone().with_text(text)?;
        Some(Self::EditText {
            after: Annotation::new(before.id, shape),
            before,
        })
    }

    pub fn crop(document: &Document, rect: Rect) -> Result<Self, CommandError> {
        let Some(after) = rect.intersection(document.full_rect()) else {
            return Err(CommandError::EmptyCrop);
        };
        if after.is_empty() {
            return Err(CommandError::EmptyCrop);
        }
        Ok(Self::Crop {
            before: document.crop(),
            after,
        })
    }

    pub fn apply(&self, document: &mut Document) -> Result<(), CommandError> {
        match self {
            Self::Add { annotation, index } => {
                document.insert(*index, annotation.clone());
                Ok(())
            }
            Self::Remove { annotation, .. } => {
                if document.take(annotation.id).is_none() {
                    return Err(CommandError::Missing);
                }
                Ok(())
            }
            Self::Move { after, .. }
            | Self::Resize { after, .. }
            | Self::Restyle { after, .. }
            | Self::EditText { after, .. } => {
                if !document.replace(after.clone()) {
                    return Err(CommandError::Missing);
                }
                Ok(())
            }
            Self::Crop { after, .. } => {
                document.set_crop(*after);
                Ok(())
            }
        }
    }

    pub fn undo(&self, document: &mut Document) -> Result<(), CommandError> {
        match self {
            Self::Add { annotation, .. } => {
                if document.take(annotation.id).is_none() {
                    return Err(CommandError::Missing);
                }
                Ok(())
            }
            Self::Remove {
                annotation, index, ..
            } => {
                document.insert(*index, annotation.clone());
                Ok(())
            }
            Self::Move { before, .. }
            | Self::Resize { before, .. }
            | Self::Restyle { before, .. }
            | Self::EditText { before, .. } => {
                if !document.replace(before.clone()) {
                    return Err(CommandError::Missing);
                }
                Ok(())
            }
            Self::Crop { before, .. } => {
                document.set_crop(*before);
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    undo: Vec<Command>,
    redo: Vec<Command>,
}

impl History {
    pub const fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn execute(
        &mut self,
        document: &mut Document,
        command: Command,
    ) -> Result<(), CommandError> {
        command.apply(document)?;
        self.undo.push(command);
        self.redo.clear();
        Ok(())
    }

    pub fn undo(&mut self, document: &mut Document) -> Result<bool, CommandError> {
        let Some(command) = self.undo.pop() else {
            return Ok(false);
        };
        if let Err(error) = command.undo(document) {
            self.undo.push(command);
            return Err(error);
        }
        self.redo.push(command);
        Ok(true)
    }

    pub fn redo(&mut self, document: &mut Document) -> Result<bool, CommandError> {
        let Some(command) = self.redo.pop() else {
            return Ok(false);
        };
        if let Err(error) = command.apply(document) {
            self.redo.push(command);
            return Err(error);
        }
        self.undo.push(command);
        Ok(true)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
