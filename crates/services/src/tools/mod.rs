use serde::{Deserialize, Serialize};
use snapline_core::actions::ActionId;
use snapline_core::combine::{Align, Background, CombineOptions, Layout};
use ts_rs::TS;

use crate::dto::tool::{
    CombineAlignDto, CombineBackgroundDto, CombineLayoutDto, CombineRequestDto, CombineSourceDto,
    EditorActionDto, EditorOutcomeDto, ToolReq, ToolResp,
};
use crate::editor::{EditorAction, EditorOutcome, EditorService, TOOL_EDITOR};
use crate::tools::combine::{CombineRequest, PickedImage, Source};
use crate::tools::pin::PinService;
use crate::{ServiceError, Services};

pub mod combine;
pub mod colour;
pub mod convert;
pub mod ocr;
pub mod pin;
pub mod qr;
pub mod ruler;
pub mod scrolling;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(rename = "ToolAreaEvent")]
#[serde(tag = "kind", content = "data", rename_all = "camelCase")]
pub enum Event {
    Ping,
}

#[derive(Debug, Clone)]
pub enum Job {
    Ping,
}

pub struct ToolService {
    _services: Services,
}

impl ToolService {
    pub(crate) fn new(services: &Services) -> Self {
        Self {
            _services: services.clone(),
        }
    }

    pub fn handle(&self, req: ToolReq) -> Result<ToolResp, ServiceError> {
        match req {
            ToolReq::Ping => Ok(ToolResp::Pong),
            ToolReq::QrMake { text } => self.qr_make(&text),
            ToolReq::QrScan { .. } => Err(ServiceError::NotYet),
            ToolReq::QrCopyText { .. } => Err(ServiceError::NotYet),
            ToolReq::QrCopyImage { text } => self.qr_copy_image(&text),
            ToolReq::QrSavePng { .. } => Err(ServiceError::NotYet),
            ToolReq::QrOpen { link } => self.qr_open(&link),
            ToolReq::ConvertPickFile => self.convert_pick_file(),
            ToolReq::ConvertStart { source, format } => self.convert_start(source, format),
            ToolReq::ConvertProgress { job_id } => self.convert_progress(job_id),
            ToolReq::ConvertCancel { job_id } => self.convert_cancel(job_id),
            ToolReq::ConvertResult { job_id } => self.convert_result(job_id),
            ToolReq::CombinePickFiles => {
                let picked = self.combine_pick_files()?;
                Ok(ToolResp::CombinePickedFiles {
                    files: picked
                        .into_iter()
                        .map(|file: PickedImage| crate::dto::tool::CombinePickedDto {
                            id: file.id,
                            name: file.name,
                        })
                        .collect(),
                })
            }
            ToolReq::CombinePreview { request } => {
                let request = to_combine(request);
                Ok(ToolResp::CombinePreviewed {
                    job: self.combine_preview(request.sources, request.options)?,
                })
            }
            ToolReq::CombineSave { request } => {
                let request = to_combine(request);
                let saved = self.combine_save(request.sources, request.options)?;
                Ok(ToolResp::CombineSaved {
                    capture_id: saved.capture_id,
                })
            }
            ToolReq::EditorOpenHistory { capture_id } => Ok(ToolResp::EditorOpened {
                outcome: editor_outcome(
                    EditorService::new(&self._services).open_history(capture_id)?,
                ),
            }),
            ToolReq::EditorOpenFile => Ok(ToolResp::EditorOpened {
                outcome: editor_outcome(EditorService::new(&self._services).open_file()?),
            }),
            ToolReq::EditorOpenClipboard => Ok(ToolResp::EditorOpened {
                outcome: editor_outcome(EditorService::new(&self._services).open_clipboard()?),
            }),
            ToolReq::PinFile => PinService::new(&self._services).file(),
            ToolReq::PinClipboard => PinService::new(&self._services).clipboard(),
            ToolReq::PinMenu => PinService::new(&self._services).menu(),
            ToolReq::Available { tool } => Ok(ToolResp::Available {
                available: pin::available(&EditorService::new(&self._services), &tool),
            }),
        }
    }
}

fn editor_outcome(outcome: EditorOutcome) -> EditorOutcomeDto {
    EditorOutcomeDto {
        action: outcome.action.map(|action| match action {
            EditorAction::Copy => EditorActionDto::Copy,
            EditorAction::Save => EditorActionDto::Save,
            EditorAction::Pin => EditorActionDto::Pin,
            EditorAction::Upload => EditorActionDto::Upload,
        }),
        capture_id: outcome.capture_id,
        path: outcome
            .path
            .map(|path| path.to_string_lossy().into_owned()),
        cancelled: outcome.cancelled,
    }
}

fn to_combine(request: CombineRequestDto) -> CombineRequest {
    CombineRequest {
        sources: request
            .sources
            .into_iter()
            .map(|source| match source {
                CombineSourceDto::History { id } => Source::History { id },
                CombineSourceDto::Picked { id } => Source::Picked { id },
            })
            .collect(),
        options: CombineOptions {
            layout: match request.options.layout {
                CombineLayoutDto::SideBySide => Layout::SideBySide,
                CombineLayoutDto::Stacked => Layout::Stacked,
            },
            spacing: request.options.spacing,
            align: match request.options.align {
                CombineAlignDto::Start => Align::Start,
                CombineAlignDto::Centre => Align::Centre,
                CombineAlignDto::End => Align::End,
            },
            background: match request.options.background {
                CombineBackgroundDto::Transparent => Background::Transparent,
                CombineBackgroundDto::White => Background::White,
                CombineBackgroundDto::Black => Background::Black,
                CombineBackgroundDto::Custom(value) => Background::Custom(value),
            },
        },
    }
}

pub fn run(action: &ActionId, services: &Services) -> Result<(), ServiceError> {
    if action.as_str() == TOOL_EDITOR {
        EditorService::new(services).open_file().map(|_| ())
    } else {
        Err(ServiceError::NotYet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_round_trips() {
        let services = Services::new();
        assert_eq!(services.tool().handle(ToolReq::Ping), Ok(ToolResp::Pong));
    }
}
