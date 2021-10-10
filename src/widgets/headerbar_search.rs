use super::search_options;
use super::search_options::SearchOptions;
use crate::search_expr;
use gtk::prelude::*;
use relm::{Component, Widget};
use relm_derive::{widget, Msg};
use std::collections::HashSet;

#[derive(Msg)]
pub enum Msg {
    SearchClicked,
    SearchActiveChanged(bool),
    SearchTextChanged(String),
    SearchTextChangedFromElsewhere((String, gdk::EventKey)),
    SearchFilterKeysChanged(HashSet<&'static str>),
}

pub struct Model {
    relm: relm::Relm<HeaderbarSearch>,
    search_toggle_signal: Option<glib::SignalHandlerId>,
    search_options: Option<Component<SearchOptions>>,
    known_filter_keys: HashSet<&'static str>,
}

#[widget]
impl Widget for HeaderbarSearch {
    fn init_view(&mut self) {
        let relm = self.model.relm.clone();
        self.model.search_toggle_signal =
            Some(self.widgets.search_toggle.connect_toggled(move |_| {
                relm.stream().emit(Msg::SearchClicked);
            }));

        self.model.search_options =
            Some(relm::init::<SearchOptions>(()).expect("Error initializing the search options"));

        let search_options_popover = gtk::PopoverBuilder::new()
            .child(self.model.search_options.as_ref().unwrap().widget())
            .build();
        self.widgets
            .search_options_btn
            .set_popover(Some(&search_options_popover));
    }

    fn model(relm: &relm::Relm<Self>, known_filter_keys: HashSet<&'static str>) -> Model {
        Model {
            relm: relm.clone(),
            search_toggle_signal: None,
            search_options: None,
            known_filter_keys,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::SearchFilterKeysChanged(hash) => {
                self.model.known_filter_keys = hash;
            }
            Msg::SearchClicked => {
                let new_visible = self.widgets.search_toggle.is_active();
                self.widgets.search_entry.grab_focus();
                self.model
                    .relm
                    .stream()
                    .emit(Msg::SearchActiveChanged(new_visible));
            }
            Msg::SearchActiveChanged(is_active) => {
                self.widgets.search_toggle.set_active(is_active);
                self.widgets.search_box.set_visible(is_active);
            }
            Msg::SearchTextChanged(_) => {
                if let Some(opt) = self.model.search_options.as_ref() {
                    let text = self.widgets.search_entry.text().to_string();
                    if text.is_empty() {
                        opt.stream()
                            .emit(search_options::Msg::EnableOptionsWithoutAndOr);
                    } else {
                        let parsed_expr =
                            search_expr::parse_search(&self.model.known_filter_keys)(&text);
                        match parsed_expr {
                            Ok(("", _)) => opt
                                .stream()
                                .emit(search_options::Msg::EnableOptionsWithAndOr),
                            _ => opt.stream().emit(search_options::Msg::DisableOptions),
                        }
                    }
                }
            }
            Msg::SearchTextChangedFromElsewhere((txt, _evt)) => {
                if !self.widgets.search_toggle.is_active() {
                    // we want to block the signal of the search button toggle,
                    // because when you click the search button we set the focus
                    // and select the search text. if we did that when search
                    // is triggered by someone typing, the first letter would
                    // be lost when typing the second letter, due to the selection
                    // so we block the search button toggle signal & handle things
                    // by hand.
                    self.widgets
                        .search_toggle
                        .block_signal(self.model.search_toggle_signal.as_ref().unwrap());
                    self.widgets.search_box.set_visible(true);
                    self.widgets.search_toggle.set_active(true);
                    self.widgets.search_entry.grab_focus_without_selecting();

                    self.widgets.search_entry.set_text(&txt);
                    self.widgets
                        .search_toggle
                        .unblock_signal(self.model.search_toggle_signal.as_ref().unwrap());
                    self.widgets.search_entry.set_position(1);
                }
            }
        }
    }

    view! {
        gtk::Box {
            #[style_class="linked"]
            #[name="search_box"]
            gtk::Box {
                visible: false,
                #[name="search_entry"]
                gtk::SearchEntry {
                    changed(entry) => Msg::SearchTextChanged(entry.text().to_string())
                },
                #[name="search_options_btn"]
                gtk::MenuButton {
                    image: Some(&gtk::Image::from_icon_name(Some("document-properties-symbolic"), gtk::IconSize::Menu)),
                    active: false,
                },
            },
            #[name="search_toggle"]
            gtk::ToggleButton {
                image: Some(&gtk::Image::from_icon_name(Some("edit-find-symbolic"), gtk::IconSize::Menu)),
                margin_start: 10,
            },
        },
    }
}
