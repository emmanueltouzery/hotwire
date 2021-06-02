// https://www.postgresql.org/docs/12/protocol.html
use crate::tshark_communication;
use quick_xml::events::Event;
use std::io::BufRead;
use std::str;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PostgresColType {
    Bool,
    Name,
    Char,
    Text,
    Oid,
    Varchar,
    Int2,
    Int4,
    Int8,
    Timestamp,
    Other,
}

#[derive(Debug)]
pub enum PostgresWireMessage {
    Startup {
        username: Option<String>,
        database: Option<String>,
        application: Option<String>,
    },
    CopyData,
    Parse {
        query: Option<String>,
        statement: Option<String>,
    },
    // for prepared statements, the first time we get parse & statement+query
    // the following times we get bind and only statement (statement ID)
    // we can then recover the query from the statement id in post-processing.
    Bind {
        statement: Option<String>,
        parameter_values: Vec<String>,
    },
    RowDescription {
        col_names: Vec<String>,
        col_types: Vec<PostgresColType>,
    },
    ResultSetRow {
        cols: Vec<String>,
    },
    ReadyForQuery,
}

pub fn parse_pgsql_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Option<PostgresWireMessage> {
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"pgsql.type") {
                    match tshark_communication::element_attr_val_string(e, b"show")
                        .unwrap()
                        .as_str()
                    {
                        "Startup message" => {
                            return Some(parse_startup_message(xml_reader));
                        }
                        "Copy data" => return Some(PostgresWireMessage::CopyData),
                        "Parse" => {
                            return Some(parse_parse_message(xml_reader));
                        },
                        "Bind" => return Some(parse_bind_message(xml_reader)),
                        "Ready for query" => {
                            return Some(PostgresWireMessage::ReadyForQuery);
                        }
                        "Row description" => {
                            return Some(parse_row_description_message(xml_reader));
                        }
                        "Data row" => return Some(parse_data_row_message(xml_reader)),
                        _ => {}
                    }
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return None;
            }
        }
    )
}

fn parse_startup_message<B: BufRead>(xml_reader: &mut quick_xml::Reader<B>) -> PostgresWireMessage {
    let mut cur_param_name = None;
    let mut username = None;
    let mut database = None;
    let mut application = None;
    let buf = &mut vec![];

    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                let val = tshark_communication::element_attr_val_string(e, b"show");
                match name.as_deref() {
                    Some(b"pgsql.parameter_name") => {
                        cur_param_name = val;
                    }
                    Some(b"pgsql.parameter_value") => match cur_param_name.as_deref() {
                        Some("user") => {
                            username = val;
                        }
                        Some("database") => {
                            database = val;
                        }
                        Some("application_name") => {
                            application = val;
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return PostgresWireMessage::Startup {
                    username,
                    database,
                    application,
                };
            }
        }
    )
}

fn parse_parse_message<B: BufRead>(xml_reader: &mut quick_xml::Reader<B>) -> PostgresWireMessage {
    let mut statement = None;
    let mut query = None;
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                match name.as_deref() {
                    Some(b"pgsql.statement") => {
                        statement = tshark_communication::element_attr_val_string(e, b"show")
                    }
                    Some(b"pgsql.query") => {
                        query = tshark_communication::element_attr_val_string(e, b"show")
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return PostgresWireMessage::Parse { statement, query };
            }
        }
        _ => {}
    )
}

fn parse_bind_message<B: BufRead>(xml_reader: &mut quick_xml::Reader<B>) -> PostgresWireMessage {
    let mut statement = None;
    let mut parameter_values = vec![];
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"pgsql.statement") {
                    statement = tshark_communication::element_attr_val_string(e, b"show")
                        .filter(|s| !s.is_empty());
                }
            }
        }
        Ok(Event::Start(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"") {
                    let show =
                        tshark_communication::element_attr_val_string(e, b"show").unwrap();
                    if show.starts_with("Parameter values") {
                        parameter_values = parse_parameter_values(xml_reader);
                    }
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return PostgresWireMessage::Bind {
                    statement,
                    parameter_values,
                };
            }
        }
    )
}

fn parse_parameter_values<B: BufRead>(xml_reader: &mut quick_xml::Reader<B>) -> Vec<String> {
    let mut param_lengths = vec![];
    let mut param_vals = vec![];
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                match name.as_deref() {
                    Some(b"pgsql.val.length") => {
                        param_lengths.push(
                            tshark_communication::element_attr_val_number(e, b"show").unwrap(),
                        );
                    }
                    Some(b"pgsql.val.data") => {
                        param_vals.push(
                            hex_chars_to_string(
                                &tshark_communication::element_attr_val_string(e, b"show")
                                    .unwrap(),
                            )
                            .unwrap_or_default(), // TODO
                        );
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"field" {
                return add_cols(param_vals, param_lengths);
            }
        }
    )
}

fn parse_row_description_message<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> PostgresWireMessage {
    let mut col_names = vec![];
    let mut col_types = vec![];
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"pgsql.oid.type")  {
                    col_types.push(parse_pg_oid_type(
                        &tshark_communication::element_attr_val_string(e, b"show").unwrap(),
                    ));
                }
            }
        }
        Ok(Event::Start(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"pgsql.col.name") {
                    col_names.push(
                        tshark_communication::element_attr_val_string(e, b"show").unwrap(),
                    );
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return PostgresWireMessage::RowDescription {
                    col_names,
                    col_types,
                };
            }
        }
    )
}

fn parse_data_row_message<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> PostgresWireMessage {
    let mut col_lengths = vec![];
    let mut col_vals = vec![];
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                match name.as_deref() {
                    Some(b"pgsql.val.length") => {
                        col_lengths.push(
                            tshark_communication::element_attr_val_number(e, b"show").unwrap(),
                        );
                    }
                    Some(b"pgsql.val.data") => {
                        col_vals.push(
                            hex_chars_to_string(
                                &tshark_communication::element_attr_val_string(e, b"show")
                                    .unwrap(),
                            )
                            // TODO the or_default is a workaround because we didn't code everything yet
                            .unwrap_or_default(),
                        );
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"field" {
                let cols = add_cols(col_vals, col_lengths);
                return PostgresWireMessage::ResultSetRow { cols };
            }
        }
    );
}

/// select * from postgres.pg_catalog.pg_type
fn parse_pg_oid_type(typ: &str) -> PostgresColType {
    match typ.parse() {
        Ok(16) => PostgresColType::Bool,
        Ok(18) => PostgresColType::Char,
        Ok(19) => PostgresColType::Name,
        Ok(20) => PostgresColType::Int8,
        Ok(21) => PostgresColType::Int2,
        Ok(23) => PostgresColType::Int4,
        Ok(25) => PostgresColType::Text,
        Ok(26) => PostgresColType::Oid,
        Ok(1043) => PostgresColType::Varchar,
        Ok(1114) => PostgresColType::Timestamp,
        _ => {
            eprintln!("Unhandled postgres type: {:?}", typ);
            PostgresColType::Other
        }
    }
}

fn add_cols(mut raw_cols: Vec<String>, col_lengths: Vec<i32>) -> Vec<String> {
    raw_cols.reverse();
    let mut cols = vec![];
    for col_length in col_lengths {
        if col_length < 0 {
            cols.push("null".to_string());
        } else if col_length == 0 {
            cols.push("".to_string());
        } else if let Some(val) = raw_cols.pop() {
            cols.push(val);
        }
    }
    if !raw_cols.is_empty() {
        panic!("raw_cols: {:?}", raw_cols);
    }
    cols
}

fn hex_chars_to_string(hex_chars: &str) -> Option<String> {
    let nocolons = hex_chars.replace(':', "");
    hex::decode(&nocolons)
        .ok()
        .map(|c| c.into_iter().collect())
        .and_then(|c: Vec<_>| {
            // the interpretation, null, digit or string is really guesswork...
            if c.first() == Some(&0u8) {
                // interpret as a number
                Some(i64::from_str_radix(&nocolons, 16).unwrap()) // i really want it to blow!
                    .map(|i| i.to_string())
            } else {
                String::from_utf8(c).ok()
            }
        })
    // hex_chars
    //     .split(':')
    //     .map(|s| u8::from_str_radix(s, 16))
    //     .join("")
}
