use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};

#[derive(Msg)]
pub enum Msg {}

pub struct HttpCommEntryData {
    pub request_verb: String,
}

pub struct Model {
    data: HttpCommEntryData,
}

#[widget]
impl Widget for HttpCommEntry {
    fn model(relm: &relm::Relm<Self>, data: HttpCommEntryData) -> Model {
        Model { data }
    }

    fn update(&mut self, event: Msg) {}

    view! {
        gtk::Grid {
            gtk::Label {
                label: &self.model.data.request_verb
            }
        }
    }
}
