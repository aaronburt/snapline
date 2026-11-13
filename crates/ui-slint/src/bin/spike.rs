use std::rc::Rc;

use slint::ComponentHandle;
use snapline_ui::Backend;

fn main() -> anyhow::Result<()> {
    let backend = Rc::new(Backend::open()?);
    let app = snapline_ui::create_window(backend, true)?;
    app.run()?;
    Ok(())
}
