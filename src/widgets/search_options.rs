use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg, Debug)]
pub enum Msg {
    TriggerAdvancedModeToggle,
    AdvancedModeWasToggled,
}

pub struct Model {}

#[widget]
impl Widget for SearchOptions {
    fn model(relm: &relm::Relm<Self>, _: ()) -> Model {
        Model {}
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::TriggerAdvancedModeToggle => {
                self.widgets.is_advanced_mode.emit_activate();
            }
            Msg::AdvancedModeWasToggled => {}
        }
    }

    view! {
         gtk::Box {
             orientation: gtk::Orientation::Horizontal,
             margin_top: 10,
             margin_start: 10,
             margin_end: 10,
             margin_bottom: 10,
             spacing: 10,
             #[name="is_advanced_mode"]
             gtk::Switch {
                 active_notify => Msg::AdvancedModeWasToggled,
             },
             gtk::EventBox {
                 gtk::Label {
                     text: "Advanced search mode",
                 },
                 button_press_event(_, _) => (Msg::TriggerAdvancedModeToggle, Inhibit(false)),
             },
         },
    }
}
