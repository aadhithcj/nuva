//! Mirror of Windows toast notifications (mail, chat, …) via UserNotificationListener.

use crate::events::{self, Event};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::core::RuntimeType;
use windows::Foundation::{AsyncStatus, IAsyncOperation, Size};
use windows::Storage::Streams::DataReader;
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

#[derive(Clone, Debug, PartialEq)]
pub struct Notif {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub body: String,
    /// UTC ticks.
    pub time: i64,
}

static ENABLED: AtomicBool = AtomicBool::new(true);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

fn convert(n: &UserNotification) -> Option<Notif> {
    let app = n.AppInfo().and_then(|a| a.DisplayInfo()).and_then(|d| d.DisplayName()).map(|h| h.to_string()).unwrap_or_default();
    let visual = n.Notification().ok()?.Visual().ok()?;
    // Modern toasts use ToastGeneric; legacy templates (ToastText02, …) have their own binding.
    let binding = KnownNotificationBindings::ToastGeneric()
        .and_then(|g| visual.GetBinding(&g))
        .ok()
        .or_else(|| visual.Bindings().ok()?.into_iter().next())?;
    let texts: Vec<String> = binding.GetTextElements().ok()?.into_iter().filter_map(|t| t.Text().ok().map(|h| h.to_string())).collect();
    let title = texts.first().cloned().unwrap_or_default();
    let body = texts.iter().skip(1).cloned().collect::<Vec<_>>().join("\n");
    Some(Notif { id: n.Id().ok()?, app, title, body, time: n.CreationTime().map(|t| t.UniversalTime).unwrap_or(0) })
}

/// Wait for a WinRT async operation, giving up after `ms` (some providers never complete).
fn wait<T: RuntimeType + 'static>(op: IAsyncOperation<T>, ms: u64) -> Option<T> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        match op.Status().ok()? {
            AsyncStatus::Completed => return op.GetResults().ok(),
            AsyncStatus::Started if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(15)),
            AsyncStatus::Started => {
                let _ = op.Cancel();
                return None;
            }
            _ => return None,
        }
    }
}

fn logo(n: &UserNotification) -> Option<Vec<u8>> {
    let r = n.AppInfo().ok()?.DisplayInfo().ok()?.GetLogo(Size { Width: 64.0, Height: 64.0 }).ok()?;
    let s = wait(r.OpenReadAsync().ok()?, 1500)?;
    let size = s.Size().ok()? as u32;
    if size == 0 || size > 4 * 1024 * 1024 {
        return None;
    }
    let rd = DataReader::CreateDataReader(&s).ok()?;
    let load: IAsyncOperation<u32> = windows::core::Interface::cast(&rd.LoadAsync(size).ok()?).ok()?;
    wait(load, 1500)?;
    let mut buf = vec![0u8; size as usize];
    rd.ReadBytes(&mut buf).ok()?;
    Some(buf)
}

pub fn start() {
    std::thread::Builder::new()
        .name("notifications".into())
        .spawn(|| unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
            let Ok(listener) = UserNotificationListener::Current() else { return };
            let mut status = listener.GetAccessStatus().unwrap_or(UserNotificationListenerAccessStatus::Denied);
            if status != UserNotificationListenerAccessStatus::Allowed {
                status = listener.RequestAccessAsync().and_then(|o| o.get()).unwrap_or(status);
            }
            crate::store::debug(&format!("notification access: {status:?}"));
            if status != UserNotificationListenerAccessStatus::Allowed {
                return;
            }
            let mut last_ids: Option<Vec<u32>> = None;
            let mut logos: HashSet<String> = HashSet::new();
            loop {
                if ENABLED.load(Ordering::SeqCst) {
                    let res = listener.GetNotificationsAsync(NotificationKinds::Toast).ok().and_then(|o| wait(o, 5000));
                    if let Some(list) = res {
                        let raw: Vec<UserNotification> = list.into_iter().collect();
                        let mut ids: Vec<u32> = raw.iter().filter_map(|n| n.Id().ok()).collect();
                        ids.sort();
                        if last_ids.as_ref() != Some(&ids) {
                            last_ids = Some(ids);
                            let mut out: Vec<Notif> = Vec::new();
                            for n in &raw {
                                if let Some(c) = convert(n) {
                                    if !logos.contains(&c.app) {
                                        logos.insert(c.app.clone());
                                        if let Some(bytes) = logo(n) {
                                            events::push(Event::NotifIcon(c.app.clone(), bytes));
                                        }
                                    }
                                    out.push(c);
                                }
                            }
                            out.sort_by(|a, b| b.time.cmp(&a.time));
                            events::push(Event::Notifications(out));
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(2500));
            }
        })
        .ok();
}
