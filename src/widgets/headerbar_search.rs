use super::search_options;
use super::search_options::Msg as SearchOptionsMsg;
use super::search_options::SearchOptions;
use crate::search_expr;
use crate::search_expr::SearchExpr;
use gtk::prelude::*;
use relm::{Component, Widget};
use relm_derive::{widget, Msg};
use std::collections::BTreeSet;

#[derive(Msg)]
pub enum Msg {
    SearchActiveChanged(bool),
    SearchTextChanged(String),
    SearchExprChanged(Option<Result<(String, search_expr::SearchExpr), String>>),
    SearchFilterKeysChanged(BTreeSet<&'static str>),
    DisplayNoSearchError,
    DisplayWithSearchErrors,
    SearchAddVals(
        (
            Option<search_options::CombineOperator>,
            &'static str,
            search_expr::SearchOperator,
            search_expr::OperatorNegation,
            String,
        ),
    ),
    RequestOptionsClose,
}

pub struct Model {
    relm: relm::Relm<HeaderbarSearch>,
    search_options: Option<Component<SearchOptions>>,
    // store the filter keys in a BTreeSet so that they'll be sorted
    // alphabetically when displaying in the GUI. An in a set because we
    // need to test for 'contains' a couple of times
    known_filter_keys: BTreeSet<&'static str>,
}

#[widget]
impl Widget for HeaderbarSearch {
    fn init_view(&mut self) {
        let relm = self.model.relm.clone();

        let so = relm::init::<SearchOptions>(self.model.known_filter_keys.clone())
            .expect("Error initializing the search options");
        relm::connect!(so@SearchOptionsMsg::Add(ref vals), self.model.relm, Msg::SearchAddVals(vals.clone()));
        relm::connect!(so@SearchOptionsMsg::AddAndCloseClick, self.model.relm, Msg::RequestOptionsClose);
        self.model.search_options = Some(so);

        let search_options_popover = gtk::PopoverBuilder::new()
            .child(self.model.search_options.as_ref().unwrap().widget())
            .build();
        self.model
            .search_options
            .as_ref()
            .unwrap()
            .stream()
            .emit(SearchOptionsMsg::ParentSet(search_options_popover.clone()));
        self.widgets
            .search_options_btn
            .set_popover(Some(&search_options_popover));
        self.update_search_status(None);
    }

    fn model(relm: &relm::Relm<Self>, known_filter_keys: BTreeSet<&'static str>) -> Model {
        Model {
            relm: relm.clone(),
            search_options: None,
            known_filter_keys,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::SearchFilterKeysChanged(hash) => {
                if let Some(so) = self.model.search_options.as_ref() {
                    so.stream()
                        .emit(search_options::Msg::FilterKeysUpdated(hash.clone()));
                }
                self.model.known_filter_keys = hash;
            }
            Msg::SearchActiveChanged(is_active) => {
                if is_active {
                    self.widgets.search_entry.grab_focus();
                }
            }
            Msg::SearchTextChanged(text) => {
                let maybe_expr = if text.is_empty() {
                    None
                } else {
                    Some(
                        search_expr::parse_search(&self.model.known_filter_keys)(&text)
                            .map(|(rest, expr)| (rest.to_string(), expr))
                            .map_err(|e| e.to_string()),
                    )
                };
                self.model
                    .relm
                    .stream()
                    .emit(Msg::SearchExprChanged(maybe_expr));
            }
            Msg::SearchExprChanged(expr) => {
                self.update_search_status(expr);
            }
            Msg::DisplayNoSearchError => {
                self.widgets
                    .search_entry
                    .set_primary_icon_name(Some("edit-find-symbolic"));
                self.widgets
                    .search_entry
                    .set_primary_icon_tooltip_text(None);
            }
            Msg::DisplayWithSearchErrors => {
                self.widgets
                    .search_entry
                    .set_primary_icon_name(Some("computer-fail-symbolic"));
                self.widgets
                    .search_entry
                    .set_primary_icon_tooltip_text(Some("Invalid search expression"));
            }
            Msg::SearchAddVals((combine_op, filter_key, search_op, op_negation, val)) => {
                let mut t = self.widgets.search_entry.text().to_string();
                match combine_op {
                    Some(search_options::CombineOperator::And) => {
                        t.push_str(" and ");
                    }
                    Some(search_options::CombineOperator::Or) => {
                        t.push_str(" or ");
                    }
                    None => {}
                }
                t.push_str(filter_key);
                match (search_op, op_negation) {
                    (
                        search_expr::SearchOperator::Contains,
                        search_expr::OperatorNegation::NotNegated,
                    ) => {
                        t.push_str(" contains ");
                    }
                    (
                        search_expr::SearchOperator::Contains,
                        search_expr::OperatorNegation::Negated,
                    ) => {
                        t.push_str(" doesntContain ");
                    }
                }
                if val.contains(' ') {
                    t.push('"');
                    t.push_str(&val.replace('"', "\""));
                    t.push('"');
                } else {
                    t.push_str(&val);
                }
                self.widgets.search_entry.set_text(&t);
            }
            Msg::RequestOptionsClose => {
                if let Some(popover) = self.widgets.search_options_btn.popover() {
                    popover.popdown();
                }
            }
        }
    }

    fn update_search_status(&mut self, maybe_expr: Option<Result<(String, SearchExpr), String>>) {
        if let Some(opt) = self.model.search_options.as_ref() {
            match maybe_expr {
                None => {
                    self.model.relm.stream().emit(Msg::DisplayNoSearchError);
                    opt.stream()
                        .emit(search_options::Msg::EnableOptionsWithoutAndOr);
                }
                Some(Ok((rest, _))) if rest.is_empty() => {
                    self.model.relm.stream().emit(Msg::DisplayNoSearchError);
                    opt.stream()
                        .emit(search_options::Msg::EnableOptionsWithAndOr);
                }
                _ => {
                    self.model.relm.stream().emit(Msg::DisplayWithSearchErrors);
                    opt.stream().emit(search_options::Msg::DisableOptions);
                }
            }
        }
    }

    view! {
        gtk::Box {
            #[style_class="linked"]
            #[name="search_box"]
            gtk::Box {
                #[name="search_entry"]
                gtk::SearchEntry {
                    hexpand: true,
                    secondary_icon_name: Some("edit-clear-symbolic"),
                    placeholder_text: Some("Enter a filter expression. Help yourself with the 'Add filter criteria' button to get started."),
                    changed(entry) => Msg::SearchTextChanged(entry.text().to_string()),
                },
                #[name="search_options_btn"]
                gtk::MenuButton {
                    image: Some(&gtk::Image::from_icon_name(Some("insert-symbolic"), gtk::IconSize::Menu)),
                    always_show_image: true,
                    label: "Add filter criteria",
                    active: false,
                },
            },
        },
    }
}
