// https://www.postgresql.org/docs/12/protocol.html
use crate::tshark_communication;
use quick_xml::events::Event;
use std::io::BufRead;
use std::str;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PostgresColType {
    Bool,
    ByteArray,
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

    // for prepared queries, it's possible the declaration
    // occured before we started recording the stream.
    // in that case we won't be able to recover the column/
    // parameter types. In that case, we have this "unknown"
    // type, for which try to guess the content type.
    Unknown,
}

impl PostgresColType {
    /// select * from postgres.pg_catalog.pg_type
    fn from_pg_oid_type(typ: &str) -> PostgresColType {
        match typ.parse() {
            Ok(16) => PostgresColType::Bool,
            Ok(17) => PostgresColType::ByteArray,
            Ok(18) => PostgresColType::Char,
            Ok(19) => PostgresColType::Name,
            Ok(20) => PostgresColType::Int8,
            Ok(21) => PostgresColType::Int2,
            Ok(23) => PostgresColType::Int4,
            Ok(25) => PostgresColType::Text,
            Ok(26) => PostgresColType::Oid,
            Ok(1043) => PostgresColType::Varchar,
            Ok(1114) => PostgresColType::Timestamp,
            _ => PostgresColType::Other,
        }
    }
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
        param_types: Vec<PostgresColType>,
    },
    // for prepared statements, the first time we get parse & statement+query
    // the following times we get bind and only statement (statement ID)
    // we can then recover the query from the statement id in post-processing.
    Bind {
        statement: Option<String>,
        parameter_lengths_and_vals: Vec<(i64, String)>,
    },
    RowDescription {
        col_names: Vec<String>,
        col_types: Vec<PostgresColType>,
    },
    ResultSetRow {
        col_lengths_and_vals: Vec<(i64, String)>,
    },
    ReadyForQuery,
}

pub fn parse_pgsql_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<Option<PostgresWireMessage>, String> {
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"pgsql.type") {
                    match tshark_communication::element_attr_val_string(e, b"show")?
                        .unwrap()
                        .as_str()
                    {
                        "Startup message" => {
                            return Ok(Some(parse_startup_message(xml_reader)?));
                        }
                        "Copy data" => return Ok(Some(PostgresWireMessage::CopyData)),
                        "Parse" => {
                            return Ok(Some(parse_parse_message(xml_reader)?));
                        },
                        "Bind" => return Ok(Some(parse_bind_message(xml_reader)?)),
                        "Ready for query" => {
                            return Ok(Some(PostgresWireMessage::ReadyForQuery));
                        }
                        "Row description" => {
                            return Ok(Some(parse_row_description_message(xml_reader)?));
                        }
                        "Data row" => return Ok(Some(parse_data_row_message(xml_reader)?)),
                        _ => {}
                    }
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return Ok(None);
            }
        }
    )
}

fn parse_startup_message<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<PostgresWireMessage, String> {
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
                let val = tshark_communication::element_attr_val_string(e, b"show")?;
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
                return Ok(PostgresWireMessage::Startup {
                    username,
                    database,
                    application,
                });
            }
        }
    )
}

fn parse_parse_message<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<PostgresWireMessage, String> {
    let mut statement = None;
    let mut query = None;
    let mut param_types = vec![];
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
                        statement = tshark_communication::element_attr_val_string(e, b"show")?
                    }
                    Some(b"pgsql.query") => {
                        query = tshark_communication::element_attr_val_string(e, b"show")?
                    }
                    _ => {}
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
                    let show = tshark_communication::element_attr_val_string(e, b"show")?;
                    if show.filter(|s| s.starts_with("Parameters: ")).is_some() {
                        param_types = parse_param_types(xml_reader)?;
                    }
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return Ok(PostgresWireMessage::Parse { statement, query, param_types });
            }
        }
    )
}

fn parse_param_types<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<Vec<PostgresColType>, String> {
    let buf = &mut vec![];
    let mut param_types = vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"pgsql.oid.type") {
                    if let Some(typ) = tshark_communication::element_attr_val_string(e, b"show")? {
                        param_types.push(PostgresColType::from_pg_oid_type(&typ));
                    }
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"field" {
                return Ok(param_types);
            }
        }
    )
}

fn parse_bind_message<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<PostgresWireMessage, String> {
    let mut statement = None;
    let mut parameter_lengths_and_vals = vec![];
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                if name.as_deref() == Some(b"pgsql.statement") {
                    statement = tshark_communication::element_attr_val_string(e, b"show")?
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
                        tshark_communication::element_attr_val_string(e, b"show")?.unwrap();
                    if show.starts_with("Parameter values") {
                        parameter_lengths_and_vals = parse_parameter_values(xml_reader)?;
                    }
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return Ok(PostgresWireMessage::Bind {
                    statement,
                    parameter_lengths_and_vals,
                });
            }
        }
    )
}

fn parse_parameter_values<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<Vec<(i64, String)>, String> {
    let mut param_length = None::<i64>;
    let mut result = vec![];
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
                        param_length =
                            tshark_communication::element_attr_val_number(e, b"show")?;
                        match param_length {
                            Some(-1) => {
                                // it's a null (-1 in the XML)
                                result.push((-1, "6e756c6c".to_string())); // null in hex
                                param_length = None;
                            }
                            Some(0) => {
                                // it's empty
                                result.push((0, "".to_string())); // null in hex
                                param_length = None;
                            }
                            _ => {}
                        }
                    }
                    Some(b"pgsql.val.data") => {
                        let val = tshark_communication::element_attr_val_string(e, b"value")?;
                        if let (Some(length), Some(parsed)) = (param_length.take(), val) {
                            result.push((length as i64, parsed));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"field" {
                return Ok(result);
            }
        }
    )
}

fn parse_row_description_message<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<PostgresWireMessage, String> {
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
                    col_types.push(PostgresColType::from_pg_oid_type(
                        &tshark_communication::element_attr_val_string(e, b"show")?.unwrap(),
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
                        tshark_communication::element_attr_val_string(e, b"show")?.unwrap(),
                    );
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return Ok(PostgresWireMessage::RowDescription {
                    col_names,
                    col_types,
                });
            }
        }
    )
}

fn parse_data_row_message<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<PostgresWireMessage, String> {
    let mut col_length = None;
    let mut col_lengths_and_vals = vec![];
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
                        col_length = tshark_communication::element_attr_val_number(e, b"show")?;
                        match col_length {
                            // TODO this is duplicated elsewhere in this file
                            Some(-1) => {
                                col_lengths_and_vals.push((-1, "6e756c6c".to_string())); // null in hex
                                col_length = None;
                            }
                            Some(0) => {
                                col_lengths_and_vals.push((0, "".to_string()));
                                col_length = None;
                            }
                            _ => {}
                        }
                    }
                    Some(b"pgsql.val.data") => {
                        let val = if let Some(v) = tshark_communication::element_attr_val_string(e, b"value")? {
                            Some(v)
                        } else {
                            tshark_communication::element_attr_val_string(e, b"show")?.map(|s| s.replace(":", ""))
                        };
                        if let (Some(length), Some(parsed_val)) = (col_length.take(), val) {
                            col_lengths_and_vals.push((length, parsed_val));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"field" {
                return Ok(PostgresWireMessage::ResultSetRow { col_lengths_and_vals });
            }
        }
    );
}
