//! A mod crate's view of the mod API: `Hooks` is implemented with the names `mod_api` exports.
//! The test is that this file builds; #7044's mod could not name `key`'s event.
use horizon_client::mod_api::{Hooks, Host, InputKeyEvent, KeyEvent, winit};

struct Keys;

impl Hooks for Keys {
    fn key(&self, _host: &mut dyn Host, event: &mut KeyEvent<'_>) {
        let _ = (event.key, event.pressed);
    }

    fn input_key(&self, _host: &mut dyn Host, event: &mut InputKeyEvent<'_>) {
        let _: &winit::event::KeyEvent = event.event;
    }
}

#[test]
fn a_mod_implements_the_key_hooks() {
    let _: Box<dyn Hooks> = Box::new(Keys);
}
