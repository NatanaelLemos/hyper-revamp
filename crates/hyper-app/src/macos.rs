//! Native macOS integration: window opacity (NSWindow.alphaValue) and the
//! system beep for `bell: "SOUND"`. No-ops on other platforms.

#[cfg(target_os = "macos")]
pub fn set_window_alpha(alpha: f32, skip_title: &str) {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    for window in app.windows().iter() {
        let title = window.title().to_string();
        if title == skip_title {
            continue;
        }
        window.setAlphaValue(alpha.clamp(0.0, 1.0) as f64);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_window_alpha(_alpha: f32, _skip_title: &str) {}

#[cfg(target_os = "macos")]
pub fn beep() {
    objc2_app_kit::NSBeep();
}

#[cfg(not(target_os = "macos"))]
pub fn beep() {}

/// Register for kAEGetURL Apple Events so ssh:// links open in the app
/// (the bundle's CFBundleURLTypes declares the scheme). Received URLs are
/// forwarded on the channel; the app opens a tab and types the ssh command
/// (Electron `open-url` → `openSSH` parity).
#[cfg(target_os = "macos")]
pub fn install_url_handler(tx: crossbeam_channel::Sender<String>) {
    use std::sync::OnceLock;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{define_class, msg_send, sel, AllocAnyThread};
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager, NSObject};

    static URL_TX: OnceLock<crossbeam_channel::Sender<String>> = OnceLock::new();

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "HyperRevampURLHandler"]
        struct UrlHandler;

        impl UrlHandler {
            #[unsafe(method(handleGetURLEvent:withReplyEvent:))]
            fn handle_get_url_event(
                &self,
                event: &NSAppleEventDescriptor,
                _reply: *mut NSAppleEventDescriptor,
            ) {
                let key_direct_object = u32::from_be_bytes(*b"----");
                let url = event
                    .paramDescriptorForKeyword(key_direct_object)
                    .and_then(|d| d.stringValue())
                    .map(|s| s.to_string());
                if let (Some(url), Some(tx)) = (url, URL_TX.get()) {
                    let _ = tx.send(url);
                }
            }
        }
    );

    if URL_TX.set(tx).is_err() {
        return; // already installed
    }
    let handler: Retained<UrlHandler> = unsafe { msg_send![UrlHandler::alloc(), init] };
    let manager = NSAppleEventManager::sharedAppleEventManager();
    let event_class = u32::from_be_bytes(*b"GURL");
    unsafe {
        let obj: &AnyObject = &handler;
        manager.setEventHandler_andSelector_forEventClass_andEventID(
            obj,
            sel!(handleGetURLEvent:withReplyEvent:),
            event_class,
            event_class,
        );
    }
    // The manager holds a weak reference; keep the handler alive for the
    // app's lifetime.
    std::mem::forget(handler);
}

#[cfg(not(target_os = "macos"))]
pub fn install_url_handler(_tx: crossbeam_channel::Sender<String>) {}

/// Claim or release the `ssh://` scheme to match the `defaultSSHApp` option,
/// as `app/index.ts` did on ready via `setAsDefaultProtocolClient`.
///
/// Only acts when the setting and reality actually disagree — reassigning a
/// handler is a user-visible system change, and on every launch macOS would
/// otherwise be told something it already knows.
#[cfg(target_os = "macos")]
pub fn sync_ssh_url_handler(should_be_default: bool) {
    use objc2_core_foundation::CFString;
    #[allow(deprecated)]
    use objc2_core_services::{LSCopyDefaultHandlerForURLScheme, LSSetDefaultHandlerForURLScheme};

    let Some(bundle_id) = bundle_identifier() else {
        // Running unbundled (`cargo run`): there is no identifier to register,
        // and LaunchServices would have nothing to point at.
        return;
    };
    let scheme = CFString::from_str("ssh");
    #[allow(deprecated)]
    let current = unsafe { LSCopyDefaultHandlerForURLScheme(&scheme) }.map(|id| id.to_string());
    let is_default = current.as_deref() == Some(bundle_id.as_str());

    if should_be_default && !is_default {
        log::info!("setting hyper-revamp as default client for ssh:// protocol");
        let handler = CFString::from_str(&bundle_id);
        #[allow(deprecated)]
        let status = unsafe { LSSetDefaultHandlerForURLScheme(&scheme, &handler) };
        if status != 0 {
            log::warn!("couldn't register as the ssh:// handler (OSStatus {status})");
        }
    } else if !should_be_default && is_default {
        // LaunchServices has no "unset" — the scheme is handed to whichever
        // other app declares it, which is what Electron's
        // `removeAsDefaultProtocolClient` did. With no candidate the setting
        // simply cannot be honored, so say so rather than failing quietly.
        match other_ssh_handler(&bundle_id) {
            Some(other) => {
                log::info!("removing hyper-revamp as default client for ssh:// protocol");
                let handler = CFString::from_str(&other);
                #[allow(deprecated)]
                let status = unsafe { LSSetDefaultHandlerForURLScheme(&scheme, &handler) };
                if status != 0 {
                    log::warn!("couldn't hand the ssh:// scheme to {other} (OSStatus {status})");
                }
            }
            None => log::warn!(
                "defaultSSHApp is false but no other app handles ssh://; leaving it registered"
            ),
        }
    }
}

/// Another installed app that declares `ssh://`, if there is one.
#[cfg(target_os = "macos")]
fn other_ssh_handler(own_bundle_id: &str) -> Option<String> {
    use objc2_core_foundation::{CFArray, CFRetained, CFString};
    #[allow(deprecated)]
    use objc2_core_services::LSCopyAllHandlersForURLScheme;

    let scheme = CFString::from_str("ssh");
    #[allow(deprecated)]
    let handlers: CFRetained<CFArray> = unsafe { LSCopyAllHandlersForURLScheme(&scheme) }?;
    for index in 0..handlers.count() {
        let value = unsafe { handlers.value_at_index(index) };
        if value.is_null() {
            continue;
        }
        let id = unsafe { CFRetained::retain(std::ptr::NonNull::new(value as *mut CFString)?) };
        let id = id.to_string();
        if id != own_bundle_id {
            return Some(id);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn bundle_identifier() -> Option<String> {
    use objc2_foundation::NSBundle;
    NSBundle::mainBundle().bundleIdentifier().map(|id| id.to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn sync_ssh_url_handler(_should_be_default: bool) {}

/// Turn an ssh:// URL into the shell command typed into a fresh tab,
/// mirroring the Electron `openSSH` action: `ssh [user@]host [-p port]`.
pub fn ssh_url_to_command(url: &str) -> Option<String> {
    let rest = url.strip_prefix("ssh://")?;
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return None;
    }
    let (user, hostport) = match rest.split_once('@') {
        Some((user, hostport)) => (Some(user), hostport),
        None => (None, rest),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
            (host, Some(port))
        }
        _ => (hostport, None),
    };
    if host.is_empty() {
        return None;
    }
    // Only allow safe characters into the typed shell line.
    let ok = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    if !ok(host) || user.is_some_and(|u| !ok(u)) {
        return None;
    }
    let mut command = String::from("ssh ");
    if let Some(user) = user {
        command.push_str(user);
        command.push('@');
    }
    command.push_str(host);
    if let Some(port) = port {
        command.push_str(" -p ");
        command.push_str(port);
    }
    command.push('\n');
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_url_parsing() {
        assert_eq!(
            ssh_url_to_command("ssh://alice@example.com:2222"),
            Some("ssh alice@example.com -p 2222\n".into())
        );
        assert_eq!(
            ssh_url_to_command("ssh://example.com"),
            Some("ssh example.com\n".into())
        );
        assert_eq!(ssh_url_to_command("ssh://bad;host"), None);
        assert_eq!(ssh_url_to_command("http://example.com"), None);
    }
}
