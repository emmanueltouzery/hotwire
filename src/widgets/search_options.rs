use crate::search_expr::{self, OperatorNegation, SearchCriteria};
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::collections::BTreeSet;

const INVALID_SEARCH_STACK_NAME: &str = "invalid-search";
const VALID_SEARCH_STACK_NAME: &str = "valid-search";

#[derive(Copy, Clone)]
pub enum CombineOperator {
    And,
    Or,
}

#[derive(Msg)]
pub enum Msg {
    ParentSet(gtk::Popover),
    FilterKeysUpdated {
        string_keys: BTreeSet<&'static str>,
        numeric_keys: BTreeSet<&'static str>,
    },
    SearchEntryKeyPress(gdk::EventKey),
    SearchTextChanged,
    AddClick,
    AddAndCloseClick,
    ClearSearchTextClick,
    Add(
        (
            Option<CombineOperator>,
            &'static str,
            SearchCriteria,
            OperatorNegation,
        ),
    ),
    DisableOptions,
    EnableOptionsWithAndOr,
    EnableOptionsWithoutAndOr,
    FilterKeyChanged,
}

pub struct Model {
    relm: relm::Relm<SearchOptions>,
    filter_keys: BTreeSet<&'static str>,
    string_filter_keys: BTreeSet<&'static str>,
    numeric_filter_keys: BTreeSet<&'static str>,
}

#[widget]
impl Widget for SearchOptions {
    fn init_view(&mut self) {
        self.widgets.and_or_combo.append_text("and");
        self.widgets.and_or_combo.append_text("or");
        self.widgets.and_or_combo.set_active(Some(0));
        for k in self.model.filter_keys.iter() {
            self.widgets.filter_key_combo.append_text(k);
        }
        self.widgets.filter_key_combo.set_active(Some(0));
        self.widgets.search_op_combo.append_text("contains");
        self.widgets.search_op_combo.append_text("doesntContain");
        self.widgets.search_op_combo.set_active(Some(0));
    }

    fn model(relm: &relm::Relm<Self>, _: ()) -> Model {
        Model {
            relm: relm.clone(),
            filter_keys: BTreeSet::new(),
            string_filter_keys: BTreeSet::new(),
            numeric_filter_keys: BTreeSet::new(),
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::ParentSet(popover) => {
                self.widgets.add_and_close_btn.set_can_default(true);
                popover.set_default_widget(Some(&self.widgets.add_and_close_btn));
            }
            Msg::FilterKeysUpdated {
                string_keys,
                numeric_keys,
            } => {
                self.model.filter_keys = string_keys.union(&numeric_keys).cloned().collect();
                self.model.string_filter_keys = string_keys;
                self.model.numeric_filter_keys = numeric_keys;
                self.widgets.filter_key_combo.remove_all();
                for k in self.model.filter_keys.iter() {
                    self.widgets.filter_key_combo.append_text(k);
                }
                self.widgets.filter_key_combo.set_active(Some(0));
            }
            Msg::SearchEntryKeyPress(e) => {
                if e.state().contains(gdk::ModifierType::CONTROL_MASK)
                    && (e.keyval() == gdk::keys::constants::Return
                        || e.keyval() == gdk::keys::constants::KP_Enter)
                {
                    self.model.relm.stream().emit(Msg::AddClick);
                }
            }
            Msg::SearchTextChanged => {
                // for string searches, we don't care about the format
                // of the string, the value is always valid. for numeric searches
                // however, validate the format of the search value
                let numeric_filter_key = self
                    .widgets
                    .filter_key_combo
                    .active_text()
                    .as_ref()
                    .and_then(|fk| self.model.numeric_filter_keys.get(fk.as_str()));
                let allow_add = if numeric_filter_key.is_some() {
                    let search_txt = self.widgets.search_entry.text().to_string();
                    Self::try_parse_filter_val_number(&search_txt).is_some()
                } else {
                    true
                };
                self.widgets.add_btn.set_sensitive(allow_add);
                self.widgets.add_and_close_btn.set_sensitive(allow_add);
            }
            Msg::DisableOptions => {
                self.widgets
                    .root_stack
                    .set_visible_child_name(INVALID_SEARCH_STACK_NAME);
            }
            Msg::EnableOptionsWithAndOr => {
                self.widgets
                    .root_stack
                    .set_visible_child_name(VALID_SEARCH_STACK_NAME);
                self.widgets.and_or_combo.set_sensitive(true);
            }
            Msg::EnableOptionsWithoutAndOr => {
                self.widgets
                    .root_stack
                    .set_visible_child_name(VALID_SEARCH_STACK_NAME);
                self.widgets.and_or_combo.set_sensitive(false);
            }
            Msg::AddClick => {
                self.add_clicked();
            }
            Msg::AddAndCloseClick => {
                self.add_clicked();
            }
            Msg::FilterKeyChanged => {
                let filter_key = self
                    .widgets
                    .filter_key_combo
                    .active_text()
                    .as_ref()
                    .map(|s| s.to_string());
                self.widgets.search_op_combo.remove_all();
                if let Some(key_str) = filter_key.as_deref() {
                    if self.model.string_filter_keys.contains(key_str) {
                        self.widgets.search_op_combo.append_text("contains");
                        self.widgets.search_op_combo.append_text("doesntContain");
                    } else {
                        self.widgets.search_op_combo.append_text(">");
                        self.widgets.search_op_combo.append_text("<");
                    }
                }
                self.widgets.search_op_combo.set_active(Some(0));

                // force refresh of the "add" and "add and close" buttons
                // being grayed out or not. Let's say the user put "hello"
                // in the text entry. The buttons should be grayed out if
                // the filter is for a numeric, but not grayed out if it's
                // for a string.
                self.model.relm.stream().emit(Msg::SearchTextChanged);
            }
            // meant for my parent
            Msg::Add(_) => {}
            Msg::ClearSearchTextClick => {}
        }
    }

    fn add_clicked(&mut self) {
        let combine_operator = if self.widgets.and_or_combo.is_sensitive() {
            Some(
                if self.widgets.and_or_combo.active_text() == Some("and".into()) {
                    CombineOperator::And
                } else {
                    CombineOperator::Or
                },
            )
        } else {
            None
        };
        let filter_key = self
            .widgets
            .filter_key_combo
            .active_text()
            .as_ref()
            .and_then(|fk| self.model.filter_keys.get(fk.as_str()))
            .unwrap();
        let search_txt = self.widgets.search_entry.text().to_string();
        if let Some((search_op, op_negation)) = match self
            .widgets
            .search_op_combo
            .active_text()
            .as_ref()
            .map(|k| k.as_str())
        {
            Some("contains") => Some((
                SearchCriteria::Contains(search_txt),
                OperatorNegation::NotNegated,
            )),
            Some("doesntContain") => Some((
                SearchCriteria::Contains(search_txt),
                OperatorNegation::Negated,
            )),
            Some(">") => Self::try_parse_filter_val_number(&search_txt)
                .map(|sc| (sc, OperatorNegation::NotNegated)),
            Some("<") => Self::try_parse_filter_val_number(&search_txt)
                .map(|sc| (sc, OperatorNegation::Negated)),
            x => panic!("unhandled search_op: {:?}", x),
        } {
            self.model.relm.stream().emit(Msg::Add((
                combine_operator,
                filter_key,
                search_op,
                op_negation,
            )));
        }
    }

    fn try_parse_filter_val_number(val: &str) -> Option<SearchCriteria> {
        match search_expr::parse_filter_val_number(val) {
            Ok(("", (n, d))) => Some(SearchCriteria::GreaterThan(n, d)),
            _ => None,
        }
    }

    view! {
        #[name="root_stack"]
        gtk::Stack {
            gtk::Grid {
                child: {
                    name: Some(VALID_SEARCH_STACK_NAME)
                },
                orientation: gtk::Orientation::Vertical,
                margin_top: 10,
                margin_start: 10,
                margin_end: 10,
                margin_bottom: 10,
                row_spacing: 5,
                column_spacing: 10,
                #[name="and_or_combo"]
                gtk::ComboBoxText {
                    cell: {
                        left_attach: 0,
                        top_attach: 0,
                    },
                },
                #[name="filter_key_combo"]
                gtk::ComboBoxText {
                    cell: {
                        left_attach: 1,
                        top_attach: 0,
                    },
                    changed => Msg::FilterKeyChanged,
                },
                #[name="search_op_combo"]
                gtk::ComboBoxText {
                    cell: {
                        left_attach: 0,
                        top_attach: 1,
                    },
                },
                #[name="search_entry"]
                gtk::SearchEntry {
                    cell: {
                        left_attach: 1,
                        top_attach: 1,
                    },
                    key_press_event(_, event) => (Msg::SearchEntryKeyPress(event.clone()), Inhibit(false)),
                    activates_default: true,
                    changed(entry) => Msg::SearchTextChanged,
                },
                gtk::ButtonBox {
                    cell: {
                        left_attach: 0,
                        top_attach: 2,
                        width: 2,
                    },
                    layout_style: gtk::ButtonBoxStyle::Expand,
                    #[name="add_btn"]
                    gtk::Button {
                        label: "Add",
                        clicked => Msg::AddClick,
                    },
                    #[name="add_and_close_btn"]
                    gtk::Button {
                        label: "Add and close",
                        can_default: true,
                        has_default: true,
                        clicked => Msg::AddAndCloseClick,
                    },
                },
            },
            gtk::Box {
                orientation: gtk::Orientation::Vertical,
                margin_top: 20,
                margin_start: 10,
                margin_end: 10,
                margin_bottom: 10,
                spacing: 10,
                child: {
                    name: Some(INVALID_SEARCH_STACK_NAME)
                },
                gtk::Label {
                    text: "Failed to parse the search string. Please correct the search string.",
                    max_width_chars: 30,
                    line_wrap: true,
                },
                #[style_class="destructive-action"]
                gtk::Button {
                    child: {
                        pack_type: gtk::PackType::End,
                    },
                    label: "Clear search text",
                    clicked => Msg::ClearSearchTextClick,
                }
            }
        },
    }
}
