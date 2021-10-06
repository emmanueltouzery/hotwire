use super::search_options::SearchOptions;
use gtk::prelude::*;
use nom;
use nom::branch::*;
use nom::bytes::complete::tag;
use nom::character::complete::*;
use nom::combinator::*;
use nom::error::*;
use nom::multi::*;
use nom::sequence::*;
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
enum SearchExpr {
    And(Box<SearchExpr>, Box<SearchExpr>),
    Or(Box<SearchExpr>, Box<SearchExpr>),
    SearchOpExpr {
        filter_key: &'static str,
        op: SearchOperator,
        filter_val: String,
    },
}

#[derive(PartialEq, Eq, Debug)]
enum SearchOperator {
    Contains,
}

fn parse_search<'a>(
    known_filter_keys: &'a HashSet<&'static str>,
) -> impl 'a + FnMut(&'a str) -> nom::IResult<&'a str, SearchExpr> {
    move |mut input: &'a str| {
        alt((
            parse_search_bracket_combinator(known_filter_keys, tag("and"), SearchExpr::And),
            parse_search_bracket_combinator(known_filter_keys, tag("or"), SearchExpr::Or),
            // TODO spaces after the bracket
            // delimited(
            //     space0,
            delimited(tag("("), parse_search(known_filter_keys), tag(")")),
            //     space0,
            // ),
            parse_search_and(known_filter_keys),
            parse_search_or(known_filter_keys),
            parse_search_expr(known_filter_keys),
        ))(input)
    }
}

fn parse_search_bracket_combinator<'a, CP: 'a, B: 'a>(
    known_filter_keys: &'a HashSet<&'static str>,
    combinator_parser: CP,
    builder: B,
) -> impl 'a + FnMut(&'a str) -> nom::IResult<&'a str, SearchExpr>
where
    CP: Fn(&'a str) -> nom::IResult<&'a str, &'a str>,
    B: Fn(Box<SearchExpr>, Box<SearchExpr>) -> SearchExpr,
{
    move |mut input: &str| {
        // TODO spaces around the bracket
        let (input, se) = delimited(tag("("), parse_search(known_filter_keys), tag(")"))(input)?;
        let (input, _) = space1(input)?;
        let (input, _) = combinator_parser(input)?;
        let (input, _) = space1(input)?;
        let (input, se2) = parse_search(known_filter_keys)(input)?;
        Ok((input, builder(Box::new(se), Box::new(se2))))
    }
}

fn parse_search_and<'a>(
    known_filter_keys: &'a HashSet<&'static str>,
) -> impl 'a + FnMut(&'a str) -> nom::IResult<&'a str, SearchExpr> {
    move |mut input: &str| {
        let (input, se) = parse_search_expr(known_filter_keys)(input)?;
        let (input, _) = space1(input)?;
        let (input, _) = tag("and")(input)?;
        let (input, _) = space1(input)?;
        let next_is_bracketed = peek::<_, _, nom::error::Error<&str>, _>(tag("("))(input).is_ok();
        let (input, se2) = parse_search(known_filter_keys)(input)?;
        match se2 {
            // we want AND to bind tighter than OR
            // so not...
            // a AND (b OR c)
            // but rather...
            // (a AND b) OR c
            // at the same time we don't want to reorder if the next expression is
            // bracketed, for instance "a and (b or c)"
            SearchExpr::Or(ose1, ose2) if !next_is_bracketed => Ok((
                input,
                SearchExpr::Or(Box::new(SearchExpr::And(Box::new(se), ose1)), ose2),
            )),
            _ => Ok((input, SearchExpr::And(Box::new(se), Box::new(se2)))),
        }
    }
}

fn parse_search_or<'a>(
    known_filter_keys: &'a HashSet<&'static str>,
) -> impl 'a + FnMut(&'a str) -> nom::IResult<&'a str, SearchExpr> {
    move |mut input: &str| {
        let (input, se) = parse_search_expr(known_filter_keys)(input)?;
        let (input, _) = space1(input)?;
        let (input, _) = tag("or")(input)?;
        let (input, _) = space1(input)?;
        let (input, se2) = parse_search(known_filter_keys)(input)?;
        Ok((input, SearchExpr::Or(Box::new(se), Box::new(se2))))
    }
}

// TODO allow negation (not X contains Y)
fn parse_search_expr<'a>(
    known_filter_keys: &'a HashSet<&'static str>,
) -> impl 'a + FnMut(&str) -> nom::IResult<&str, SearchExpr> {
    move |mut input: &str| {
        let (input, filter_key) = parse_filter_key(known_filter_keys.clone())(input)?;
        let (input, _) = space1(input)?;
        let (input, op) = parse_filter_op(input)?;
        let (input, _) = space1(input)?;
        let (input, filter_val) = parse_filter_val(input)?;
        Ok((
            input,
            SearchExpr::SearchOpExpr {
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
    // i want to allow unicode characters here, so alphanumeric is not good enough, I think
    fold_many1(
        none_of(" \t\r\n()\",;:!?/*+"),
        String::new,
        |mut sofar, cur| {
            sofar.push(cur);
            sofar
        },
    )(input)
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
            parse_search(&["detail.contents"].iter().cloned().collect())(
                "grid.cells contains test"
            )
            .is_err()
        );
    }

    #[test]
    fn parse_combined_search_expression() {
        assert_eq!(
            (
                "",
                (SearchExpr::Or(
                    Box::new(SearchExpr::And(
                        Box::new(SearchExpr::SearchOpExpr {
                            filter_key: "grid.cells",
                            op: SearchOperator::Contains,
                            filter_val: "test".to_string(),
                        }),
                        Box::new(SearchExpr::SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            filter_val: "details val".to_string(),
                        }),
                    )),
                    Box::new(SearchExpr::SearchOpExpr {
                        filter_key: "detail.contents",
                        op: SearchOperator::Contains,
                        filter_val: "val2".to_string(),
                    }),
                ))
            ),
            parse_search(
                &["grid.cells", "detail.contents", "other"]
                    .iter()
                    .cloned()
                    .collect()
            )("grid.cells contains test and detail.contents contains \"details val\" or detail.contents contains val2")
            .unwrap()
        );
    }

    #[test]
    fn parse_combined_search_expression_with_brackets() {
        assert_eq!(
            Ok((
                "",
                (SearchExpr::And(
                    Box::new(SearchExpr::SearchOpExpr {
                        filter_key: "grid.cells",
                        op: SearchOperator::Contains,
                        filter_val: "test".to_string(),
                    }),
                    Box::new(SearchExpr::Or(
                        Box::new(SearchExpr::SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            filter_val: "details val".to_string(),
                        }),
                        Box::new(SearchExpr::SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            filter_val: "val2".to_string(),
                        }),
                    ))
                ))
            )),
            parse_search(
                &["grid.cells", "detail.contents", "other"]
                    .iter()
                    .cloned()
                    .collect()
            )(
                "grid.cells contains test and (detail.contents contains \"details val\" or detail.contents contains val2)"
            )
        );
    }

    #[test]
    fn parse_combined_search_expression_with_brackets2() {
        assert_eq!(
            (
                "",
                (SearchExpr::Or(
                    Box::new(SearchExpr::And(
                        Box::new(SearchExpr::SearchOpExpr {
                            filter_key: "grid.cells",
                            op: SearchOperator::Contains,
                            filter_val: "test".to_string(),
                        }),
                        Box::new(SearchExpr::SearchOpExpr {
                            filter_key: "detail.contents",
                            op: SearchOperator::Contains,
                            filter_val: "details val".to_string(),
                        }),
                    )),
                    Box::new(SearchExpr::SearchOpExpr {
                        filter_key: "detail.contents",
                        op: SearchOperator::Contains,
                        filter_val: "val2".to_string(),
                    }),
                ))
            ),
            parse_search(
                &["grid.cells", "detail.contents", "other"]
                    .iter()
                    .cloned()
                    .collect()
            )("(grid.cells contains test and detail.contents contains \"details val\") or detail.contents contains val2")
            .unwrap()
        );
    }
}
