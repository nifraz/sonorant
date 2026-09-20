//! Keeping the screen on while something is worth watching.
//!
//! A full-screen analyser playing music has no keypresses and no pointer movement, so
//! the desktop would blank it. The XDG inhibit portal is how an application says
//! otherwise, and it works the same inside a Flatpak and out of one. The inhibition
//! lasts as long as the request object does, so this keeps its path and closes it to let
//! the screen go again.

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INHIBIT: &str = "org.freedesktop.portal.Inhibit";
const REQUEST: &str = "org.freedesktop.portal.Request";
/// The portal's flag for "don't let the session idle".
const IDLE: u32 = 8;

/// Holds the screen on while it is asked to.
#[derive(Debug, Default)]
pub struct ScreenAwake {
    connection: Option<Connection>,
    /// The portal request holding the inhibition, while there is one.
    request: Option<OwnedObjectPath>,
}

impl ScreenAwake {
    pub fn new() -> ScreenAwake {
        ScreenAwake::default()
    }

    /// Asks for the screen to stay on, or lets it go back to the usual timeout.
    pub fn set(&mut self, on: bool) {
        if on == self.request.is_some() {
            return;
        }
        let result = if on { self.hold() } else { self.release() };
        if let Err(e) = result {
            // A desktop with no portal is a reason to carry on without the inhibition,
            // not a reason to stop.
            log::debug!(
                "cannot {} the screen awake: {e}",
                if on { "hold" } else { "let go of" }
            );
        }
    }

    fn hold(&mut self) -> zbus::Result<()> {
        let connection = self.connect()?;
        let proxy = Proxy::new(&connection, PORTAL, PATH, INHIBIT)?;
        // No parent window: Sonorant's is not an X11 or Wayland handle the portal can
        // use, and the reason is what the desktop shows if it asks.
        let options: Vec<(&str, Value<'_>)> =
            vec![("reason", Value::new("Sonorant is showing what is playing"))];
        let request: OwnedObjectPath = proxy.call("Inhibit", &("", IDLE, options))?;
        log::debug!("screen awake: held by {request}");
        self.request = Some(request);
        Ok(())
    }

    fn release(&mut self) -> zbus::Result<()> {
        let Some(request) = self.request.take() else {
            return Ok(());
        };
        let connection = self.connect()?;
        let path: ObjectPath<'_> = request.as_ref();
        let proxy = Proxy::new(&connection, PORTAL, path, REQUEST)?;
        proxy.call::<_, _, ()>("Close", &())?;
        log::debug!("screen awake: let go");
        Ok(())
    }

    /// The session bus, opened once and kept: the inhibition lasts as long as the
    /// connection that asked for it, so this cannot be a connection per call.
    fn connect(&mut self) -> zbus::Result<Connection> {
        if let Some(c) = &self.connection {
            return Ok(c.clone());
        }
        let c = Connection::session()?;
        self.connection = Some(c.clone());
        Ok(c)
    }
}

impl Drop for ScreenAwake {
    fn drop(&mut self) {
        self.set(false);
    }
}
