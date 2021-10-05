use super::search_options::SearchOptions;
use gtk::prelude::*;
use nom;
use nom::branch::*;
use nom::bytes::complete::tag;
use nom::character::complete::*;
use nom::combinator::*;
use nom::error::*;
use nom::multi::*;
use nom::Err;
use relm::{Component, Widget};
use relm_derive::{widget, Msg};
use std::collections::HashSet;

#[derive(Msg)]
pub enum Msg {
    SearchClicked,
    SearchActiveChanged(bool),
    SearchTextChanged(String),
    SearchTextChangedFromElsewhere((String, gdk::EventKey)),
}

pub struct Model {
    relm: relm::Relm<HeaderbarSearch>,
    search_toggle_signal: Option<glib::SignalHandlerId>,
    search_options: Option<Component<SearchOptions>>,
}

#[derive(PartialEq, Eq, Debug)]
enum SearchOperator {
    Contains,
}

#[derive(PartialEq, Eq, Debug)]
struct SearchExpr {
    filter_key: &'static str,
    op: SearchOperator,
    filter_val: String,
}

#[derive(PartialEq, Eq, Debug)]
enum SearchCombinator {
    And,
    Or,
}

fn parse_search(
    known_filter_keys: HashSet<&'static str>,
) -> impl FnMut(&str) -> nom::IResult<&str, (SearchExpr, Vec<(SearchCombinator, SearchExpr)>)> {
    move |mut input: &str| {
        let (input, se) = parse_search_expr(known_filter_keys.clone())(input)?;
        let (input, rest) = many1(parse_extra_search_expr(known_filter_keys.clone()))(input)?;
        Ok((input, (se, rest)))
    }
}

fn parse_extra_search_expr(
    known_filter_keys: HashSet<&'static str>,
) -> impl FnMut(&str) -> nom::IResult<&str, (SearchCombinator, SearchExpr)> {
    move |mut input: &str| {
        let (input, combinator) = parse_search_combinator(input)?;
        let (input, _) = space1(input)?;
        let (input, search_expr) = parse_search_expr(known_filter_keys.clone())(input)?;
        Ok((input, (combinator, search_expr)))
    }
}

fn parse_search_combinator(input: &str) -> nom::IResult<&str, SearchCombinator> {
    let (input, t) = alt((tag("and"), tag("or")))(input)?;
    let comb = match t {
        "and" => SearchCombinator::And,
        "or" => SearchCombinator::Or,
        _ => panic!(), // ####
    };
    Ok((input, comb))
}

// TODO allow negation (not X contains Y)
fn parse_search_expr(
    known_filter_keys: HashSet<&'static str>,
) -> impl FnMut(&str) -> nom::IResult<&str, SearchExpr> {
    move |mut input: &str| {
        let (input, filter_key) = parse_filter_key(known_filter_keys.clone())(input)?;
        let (input, _) = space1(input)?;
        let (input, op) = parse_filter_op(input)?;
        let (input, _) = space1(input)?;
        let (input, filter_val) = parse_filter_val(input)?;
        let (input, _) = space0(input)?; // eat trailing spaces
        Ok((
            input,
            SearchExpr {
                filter_key,
                op,
                filter_val,
            },
        ))
    }
}

fn parse_filter_val(input: &str) -> nom::IResult<&str, String> {
    alt((parse_quoted_string, parse_word))(input)
}

fn parse_filter_op(input: &str) -> nom::IResult<&str, SearchOperator> {
    let (input, _t) = tag("contains")(input)?;
    Ok((input, SearchOperator::Contains))
}

fn parse_filter_key(
    known_filter_keys: HashSet<&'static str>,
) -> impl FnMut(&str) -> nom::IResult<&str, &'static str> {
    move |mut input: &str| {
        // let (input, filter_key) = recognize(parse_filter_key_basic)(input)?;
        map_res(recognize(parse_filter_key_basic), |s: &str| {
            known_filter_keys
                .get(s)
                .map(|s| *s)
                .ok_or(Err::Error(("bad filter key", ErrorKind::Verify)))
        })(input)
    }
}

fn parse_filter_key_basic(input: &str) -> nom::IResult<&str, ()> {
    let (input, _) = alpha1(input)?;
    let (input, _) = char('.')(input)?;
    let (input, _) = alpha1(input)?;
    Ok((input, ()))
}

fn parse_quoted_string(input: &str) -> nom::IResult<&str, String> {
    let (input, _) = char('"')(input)?;
    let (input, st) = fold_many0(quoted_string_char, String::new, |mut sofar, cur| {
        sofar.push(cur);
        sofar
    })(input)?;
    let (input, _) = char('"')(input)?;
    Ok((input, st))
}

fn quoted_string_char(input: &str) -> nom::IResult<&str, char> {
    alt((none_of("\\\""), escaped_char))(input)
}

// meant for \" mostly for now
fn escaped_char(input: &str) -> nom::IResult<&str, char> {
    let (input, _) = char('\\')(input)?;
    none_of("\\")(input)
}

fn parse_word(input: &str) -> nom::IResult<&str, String> {
    // input.split_at_position1_complete(|item| {
    //     item == ' ' || item == '\t' || item == '\r' || item == '\n'
    // })
    fold_many1(none_of(" \t\r\n"), String::new, |mut sofar, cur| {
        sofar.push(cur);
        sofar
    })(input)
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

    fn model(relm: &relm::Relm<Self>, _: ()) -> Model {
        Model {
            relm: relm.clone(),
            search_toggle_signal: None,
            search_options: None,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
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
            Msg::SearchTextChanged(_) => {} // meant for my parent
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quoted_string_simple_case() {
        assert_eq!(
            "my \"string",
            parse_quoted_string("\"my \\\"string\"").unwrap().1
        );
    }

    #[test]
    fn should_reject_unknown_filter_key() {
        assert_eq!(
            true,
            parse_search(["detail.contents"].iter().cloned().collect())("grid.cells contains test")
                .is_err()
        );
    }

    #[test]
    fn parse_combined_search_expression() {
        assert_eq!(
            (
                "",
                (
                    SearchExpr {
                        filter_key: "grid.cells",
                        op: SearchOperator::Contains,
                        filter_val: "test".to_string(),
                    },
                    vec![(
                        SearchCombinator::And,
                        SearchExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            filter_val: "details val".to_string(),
                        }
                    )]
                )
            ),
            parse_search(
                ["grid.cells", "detail.contents", "other"]
                    .iter()
                    .cloned()
                    .collect()
            )("grid.cells contains test and detail.contents contains \"details val\"")
            .unwrap()
        );
    }
}
