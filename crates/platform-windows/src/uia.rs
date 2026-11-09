use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use snapline_core::geometry::{Point, Px, Rect};
use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

pub const DEADLINE: Duration = Duration::from_millis(30);

pub trait ElementQuery: 'static {
    fn rect_at(&self, point: Point) -> Option<Rect>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlQuery {
    pub generation: u64,
    pub point: Point,
    created: Instant,
}

impl ControlQuery {
    pub fn new(generation: u64, point: Point) -> Self {
        Self {
            generation,
            point,
            created: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.created.elapsed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlReply {
    Found { generation: u64, rect: Option<Rect> },
    TimedOut { generation: u64 },
}

impl ControlReply {
    pub const fn generation(&self) -> u64 {
        match self {
            Self::Found { generation, .. } | Self::TimedOut { generation } => *generation,
        }
    }

    pub fn rect_for(&self, generation: u64) -> Option<Option<Rect>> {
        if self.generation() != generation {
            return None;
        }
        match self {
            Self::Found { rect, .. } => Some(*rect),
            Self::TimedOut { .. } => Some(None),
        }
    }
}

pub struct ControlSnapWorker {
    requests: Sender<ControlQuery>,
    replies: Receiver<ControlReply>,
}

impl ControlSnapWorker {
    pub fn spawn() -> Self {
        Self::spawn_with(DEADLINE, UiaQuery::new)
    }

    pub fn spawn_with<Q, F>(deadline: Duration, make: F) -> Self
    where
        Q: ElementQuery,
        F: FnOnce() -> Option<Q> + Send + 'static,
    {
        let (requests, request_receiver) = mpsc::channel();
        let (reply_sender, replies) = mpsc::channel();
        let _ = std::thread::Builder::new()
            .name(String::from("snapline-control-snap"))
            .spawn(move || run_worker(deadline, make, &request_receiver, &reply_sender));
        Self { requests, replies }
    }

    pub fn query(&self, generation: u64, point: Point) -> bool {
        self.requests
            .send(ControlQuery::new(generation, point))
            .is_ok()
    }

    pub fn poll(&self) -> Option<ControlReply> {
        self.replies.try_recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Option<ControlReply> {
        self.replies.recv_timeout(timeout).ok()
    }
}

fn run_worker<Q, F>(
    deadline: Duration,
    make: F,
    requests: &Receiver<ControlQuery>,
    replies: &Sender<ControlReply>,
) where
    Q: ElementQuery,
    F: FnOnce() -> Option<Q>,
{
    let _com = ComGuard::new();
    // SAFETY: sets the DPI awareness of this dedicated worker thread before any query.
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let query = make();
    while let Ok(mut request) = requests.recv() {
        while let Ok(next) = requests.try_recv() {
            request = next;
        }
        if request.elapsed() >= deadline {
            let _ = replies.send(ControlReply::TimedOut {
                generation: request.generation,
            });
            continue;
        }
        let rect = query
            .as_ref()
            .and_then(|query| query.rect_at(request.point));
        let reply = if request.elapsed() > deadline {
            ControlReply::TimedOut {
                generation: request.generation,
            }
        } else {
            ControlReply::Found {
                generation: request.generation,
                rect,
            }
        };
        let _ = replies.send(reply);
    }
}

struct UiaQuery {
    automation: IUIAutomation,
}

impl UiaQuery {
    fn new() -> Option<Self> {
        // SAFETY: CUIAutomation is a registered in-process COM class and the returned interface owns a reference.
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
        Some(Self { automation })
    }
}

impl ElementQuery for UiaQuery {
    fn rect_at(&self, point: Point) -> Option<Rect> {
        let native = POINT {
            x: point.x.get(),
            y: point.y.get(),
        };
        // SAFETY: the automation object is live and the point is a plain value.
        let element = unsafe { self.automation.ElementFromPoint(native) }.ok()?;
        // SAFETY: the element interface is live for the duration of the call.
        let offscreen = unsafe { element.CurrentIsOffscreen() }
            .map(|value| value.as_bool())
            .unwrap_or(false);
        if offscreen {
            return None;
        }
        // SAFETY: the element interface is live and the rectangle is a live out-value.
        let rect = unsafe { element.CurrentBoundingRectangle() }.ok()?;
        rect_from(rect)
    }
}

fn rect_from(rect: RECT) -> Option<Rect> {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return None;
    }
    Some(Rect::new(
        Px(rect.left),
        Px(rect.top),
        Px(rect.right - rect.left),
        Px(rect.bottom - rect.top),
    ))
}

struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn new() -> Self {
        // SAFETY: initialises COM for the dedicated worker thread with a multithreaded apartment.
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self {
            initialized: status.is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: balances the successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    struct FakeQuery {
        rect: Option<Rect>,
        delay: Duration,
    }

    impl ElementQuery for FakeQuery {
        fn rect_at(&self, _point: Point) -> Option<Rect> {
            if !self.delay.is_zero() {
                thread::sleep(self.delay);
            }
            self.rect
        }
    }

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect::new(Px(x), Px(y), Px(width), Px(height))
    }

    fn point(x: i32, y: i32) -> Point {
        Point::new(Px(x), Px(y))
    }

    #[test]
    fn worker_returns_the_control_rectangle() {
        let worker = ControlSnapWorker::spawn_with(DEADLINE, || {
            Some(FakeQuery {
                rect: Some(rect(10, 20, 30, 40)),
                delay: Duration::ZERO,
            })
        });
        assert!(worker.query(7, point(15, 25)));
        assert_eq!(
            worker.recv_timeout(Duration::from_secs(2)),
            Some(ControlReply::Found {
                generation: 7,
                rect: Some(rect(10, 20, 30, 40)),
            })
        );
    }

    #[test]
    fn worker_reports_an_empty_element_as_no_rectangle() {
        let worker = ControlSnapWorker::spawn_with(DEADLINE, || {
            Some(FakeQuery {
                rect: None,
                delay: Duration::ZERO,
            })
        });
        assert!(worker.query(4, point(0, 0)));
        assert_eq!(
            worker.recv_timeout(Duration::from_secs(2)),
            Some(ControlReply::Found {
                generation: 4,
                rect: None,
            })
        );
    }

    #[test]
    fn late_answers_are_dropped_as_timeouts() {
        let worker = ControlSnapWorker::spawn_with(Duration::from_millis(5), || {
            Some(FakeQuery {
                rect: Some(rect(1, 2, 3, 4)),
                delay: Duration::from_millis(80),
            })
        });
        assert!(worker.query(9, point(5, 5)));
        assert_eq!(
            worker.recv_timeout(Duration::from_secs(2)),
            Some(ControlReply::TimedOut { generation: 9 })
        );
    }

    #[test]
    fn replies_from_another_generation_are_ignored() {
        let found = ControlReply::Found {
            generation: 2,
            rect: Some(rect(0, 0, 10, 10)),
        };
        assert_eq!(found.rect_for(2), Some(Some(rect(0, 0, 10, 10))));
        assert_eq!(found.rect_for(1), None);
        let timeout = ControlReply::TimedOut { generation: 3 };
        assert_eq!(timeout.rect_for(3), Some(None));
        assert_eq!(timeout.rect_for(4), None);
    }

    #[test]
    fn a_worker_without_a_query_backend_reports_no_rectangle() {
        let worker = ControlSnapWorker::spawn_with(DEADLINE, || None::<FakeQuery>);
        assert!(worker.query(1, point(0, 0)));
        assert_eq!(
            worker.recv_timeout(Duration::from_secs(2)),
            Some(ControlReply::Found {
                generation: 1,
                rect: None,
            })
        );
    }
}
