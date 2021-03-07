use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg)]
pub enum Msg {}

pub struct Model {
    request_verb: String,
}

#[widget]
impl Widget for HttpCommEntry {
    fn model(relm: &relm::Relm<Self>, request_verb: String) -> Model {
        Model { request_verb }
    }

    fn update(&mut self, event: Msg) {}

    view! {
        gtk::Grid {
            gtk::Label {
                label: &self.model.request_verb
            }
        }
    }
}
