use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiApartment {
    Ready,
    AlreadyMultithreaded,
    Failed,
}

pub fn init_ui_thread() -> UiApartment {
    // SAFETY: no pointer is passed and the apartment is kept for the life of the calling thread.
    let status = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if status.is_ok() {
        UiApartment::Ready
    } else if status == RPC_E_CHANGED_MODE {
        UiApartment::AlreadyMultithreaded
    } else {
        UiApartment::Failed
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::Com::{CoUninitialize, COINIT_MULTITHREADED};
    use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};

    use super::*;

    #[test]
    fn a_ui_thread_keeps_its_apartment_when_winrt_asks_for_multithreaded() {
        let outcome = std::thread::spawn(|| {
            let apartment = init_ui_thread();
            // SAFETY: no pointer is passed and the call only reports the existing apartment.
            let later = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            // SAFETY: no pointer is passed and the thread already owns a single-threaded apartment.
            let ole = unsafe { OleInitialize(None) };
            if ole.is_ok() {
                // SAFETY: balances the successful OleInitialize above on this thread.
                unsafe { OleUninitialize() };
            }
            // SAFETY: balances the successful CoInitializeEx in init_ui_thread on this thread.
            unsafe { CoUninitialize() };
            (apartment, later == RPC_E_CHANGED_MODE, ole.is_ok())
        })
        .join()
        .expect("thread");
        assert_eq!(outcome, (UiApartment::Ready, true, true));
    }

    #[test]
    fn a_multithreaded_thread_is_reported_and_left_alone() {
        let outcome = std::thread::spawn(|| {
            // SAFETY: no pointer is passed and the matching CoUninitialize runs below.
            let first = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let apartment = init_ui_thread();
            if first.is_ok() {
                // SAFETY: balances the successful CoInitializeEx above on this thread.
                unsafe { CoUninitialize() };
            }
            apartment
        })
        .join()
        .expect("thread");
        assert_eq!(outcome, UiApartment::AlreadyMultithreaded);
    }
}
