// SPDX-License-Identifier: AGPL-3.0-only

//! `CardDAV` and `CalDAV`, so phone and desktop address books and calendars can
//! sync with a space.
//!
//! The resources stay JSON (`contact/<id>`, `event/<id>`); this module only
//! converts at the edge.
//!
//! ```text
//! /dav/                          the principal
//! /dav/contacts/                 address book home: one book per space
//! /dav/contacts/{space}/{id}.vcf a contact
//! /dav/calendars/                calendar home: one calendar per space
//! /dav/calendars/{space}/{id}.ics an event
//! ```
//!
//! Collections carry a `getctag` (the space's sequence number); items carry
//! the `ETag` of the stored JSON. Queries ignore filters and return every
//! item, which clients accept.

use std::collections::BTreeSet;

use hs_core::Hash;
use hs_node::Access;
use http::{HeaderValue, Method, StatusCode, header};
use quick_xml::NsReader;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;

use crate::util::{
    ApiError, Result, empty, escape, header as hdr, percent_decode, percent_encode, respond,
    set_header,
};
use crate::{Api, formats, rest};
use hs_net::server::{Request, Response};

const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";
const CS: &str = "http://calendarserver.org/ns/";

/// Address books or calendars.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Book {
    Contacts,
    Calendars,
}

impl Book {
    fn parse(s: &str) -> Option<Book> {
        match s {
            "contacts" => Some(Book::Contacts),
            "calendars" => Some(Book::Calendars),
            _ => None,
        }
    }
    fn path(self) -> &'static str {
        match self {
            Book::Contacts => "contacts",
            Book::Calendars => "calendars",
        }
    }
    fn kind(self) -> &'static str {
        match self {
            Book::Contacts => "contact",
            Book::Calendars => "event",
        }
    }
    fn ext(self) -> &'static str {
        match self {
            Book::Contacts => ".vcf",
            Book::Calendars => ".ics",
        }
    }
    fn media_type(self) -> &'static str {
        match self {
            Book::Contacts => "text/vcard; charset=utf-8",
            Book::Calendars => "text/calendar; charset=utf-8",
        }
    }
    fn render(self, v: &serde_json::Value, target: &str) -> String {
        match self {
            Book::Contacts => formats::to_vcard(v, target),
            Book::Calendars => formats::to_ical(v, target),
        }
    }
}

/// What a path names.
enum Res {
    Root,
    Home(Book),
    Collection {
        book: Book,
        id: Hash,
        name: String,
        seq: u64,
    },
    Item {
        book: Book,
        id: Hash,
        stem: String,
        etag: String,
        data: Option<String>,
    },
}

impl Res {
    fn href(&self) -> String {
        match self {
            Res::Root => "/dav/".into(),
            Res::Home(b) => format!("/dav/{}/", b.path()),
            Res::Collection { book, id, .. } => format!("/dav/{}/{}/", book.path(), id.to_hex()),
            Res::Item { book, id, stem, .. } => {
                format!(
                    "/dav/{}/{}/{}{}",
                    book.path(),
                    id.to_hex(),
                    percent_encode(stem, false),
                    book.ext()
                )
            }
        }
    }
}

/// A property name: namespace and local name.
type Prop = (String, String);

fn prop(ns: &str, name: &str) -> Prop {
    (ns.to_string(), name.to_string())
}

/// What a PROPFIND or REPORT body asks for.
#[derive(Default)]
struct Ask {
    report: Option<String>,
    all: bool,
    props: Vec<Prop>,
    hrefs: Vec<String>,
}

fn parse_body(body: &[u8]) -> Ask {
    let mut ask = Ask::default();
    let text = String::from_utf8_lossy(body);
    if text.trim().is_empty() {
        ask.all = true;
        return ask;
    }
    let mut r = NsReader::from_str(&text);
    let mut stack: Vec<String> = Vec::new();
    let mut href = String::new();
    while let Ok((ns, ev)) = r.read_resolved_event() {
        let ns = match ns {
            ResolveResult::Bound(n) => n.0.to_string(),
            _ => String::new(),
        };
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let local = e.local_name().as_ref().to_string();
                if stack.is_empty() {
                    ask.report = Some(local.clone());
                }
                if local == "allprop" {
                    ask.all = true;
                } else if stack.last().is_some_and(|p| p == "prop") {
                    ask.props.push((ns, local.clone()));
                }
                if matches!(ev, Event::Start(_)) {
                    stack.push(local);
                }
            }
            Event::End(_) => {
                if stack.pop().as_deref() == Some("href") {
                    ask.hrefs.push(std::mem::take(&mut href));
                }
            }
            Event::Text(t) if stack.last().is_some_and(|p| p == "href") => {
                href.push_str(&t.xml10_content());
            }
            Event::GeneralRef(g) if stack.last().is_some_and(|p| p == "href") => {
                match g.resolve_char_ref() {
                    Ok(Some(c)) => href.push(c),
                    _ => href.push_str(
                        quick_xml::escape::resolve_xml_entity(&g.xml10_content()).unwrap_or(""),
                    ),
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if ask.props.is_empty() && ask.report.as_deref() == Some("propfind") {
        ask.all = true;
    }
    ask
}

fn prefix(ns: &str) -> Option<&'static str> {
    match ns {
        DAV => Some("d"),
        CALDAV => Some("c"),
        CARDDAV => Some("a"),
        CS => Some("cs"),
        _ => None,
    }
}

fn element(p: &Prop, inner: &str) -> String {
    let (ns, name) = p;
    match prefix(ns) {
        Some(x) if inner.is_empty() => format!("<{x}:{name}/>"),
        Some(x) => format!("<{x}:{name}>{inner}</{x}:{name}>"),
        None if inner.is_empty() => format!("<x:{name} xmlns:x=\"{}\"/>", escape(ns)),
        None => format!("<x:{name} xmlns:x=\"{}\">{inner}</x:{name}>", escape(ns)),
    }
}

fn href(h: &str) -> String {
    format!("<d:href>{}</d:href>", escape(h))
}

/// A property's value, or `None` if this resource does not have it.
fn value(api: &Api, res: &Res, p: &Prop) -> Option<String> {
    let (ns, name) = (p.0.as_str(), p.1.as_str());
    let collection = matches!(res, Res::Root | Res::Home(_) | Res::Collection { .. });
    Some(match (ns, name) {
        (DAV, "resourcetype") => match res {
            Res::Root => "<d:collection/><d:principal/>".into(),
            Res::Home(_) => "<d:collection/>".into(),
            Res::Collection { book: Book::Contacts, .. } => "<d:collection/><a:addressbook/>".into(),
            Res::Collection { book: Book::Calendars, .. } => "<d:collection/><c:calendar/>".into(),
            Res::Item { .. } => String::new(),
        },
        (DAV, "displayname") => match res {
            Res::Root => escape(&api.node.config().name),
            Res::Home(Book::Contacts) => "Contacts".into(),
            Res::Home(Book::Calendars) => "Calendars".into(),
            Res::Collection { name, .. } => escape(name),
            Res::Item { .. } => return None,
        },
        (DAV, "current-user-principal" | "principal-URL" | "owner")
        | (CALDAV, "calendar-user-address-set") => href("/dav/"),
        (CARDDAV, "addressbook-home-set") => href("/dav/contacts/"),
        (CALDAV, "calendar-home-set") => href("/dav/calendars/"),
        (DAV, "current-user-privilege-set") => {
            "<d:privilege><d:read/></d:privilege><d:privilege><d:write/></d:privilege><d:privilege><d:write-content/></d:privilege><d:privilege><d:bind/></d:privilege><d:privilege><d:unbind/></d:privilege>".into()
        }
        (DAV, "supported-report-set") if collection => match res {
            Res::Collection { book: Book::Contacts, .. } => {
                "<d:supported-report><d:report><a:addressbook-multiget/></d:report></d:supported-report><d:supported-report><d:report><a:addressbook-query/></d:report></d:supported-report>".into()
            }
            Res::Collection { book: Book::Calendars, .. } => {
                "<d:supported-report><d:report><c:calendar-multiget/></d:report></d:supported-report><d:supported-report><d:report><c:calendar-query/></d:report></d:supported-report>".into()
            }
            _ => String::new(),
        },
        (CALDAV, "supported-calendar-component-set") if matches!(res, Res::Collection { book: Book::Calendars, .. }) => {
            "<c:comp name=\"VEVENT\"/>".into()
        }
        (CARDDAV, "supported-address-data") if matches!(res, Res::Collection { book: Book::Contacts, .. }) => {
            "<a:address-data-type content-type=\"text/vcard\" version=\"3.0\"/>".into()
        }
        (CS, "getctag") | (DAV, "sync-token") => match res {
            Res::Collection { seq, .. } if name == "getctag" => format!("\"{seq}\""),
            Res::Collection { id, seq, .. } => format!("urn:homeserver:{}:{seq}", id.to_hex()),
            _ => return None,
        },
        (DAV, "getetag") => match res {
            Res::Item { etag, .. } => escape(etag),
            _ => return None,
        },
        (DAV, "getcontenttype") => match res {
            Res::Item { book, .. } => book.media_type().into(),
            _ => "httpd/unix-directory".into(),
        },
        (CARDDAV, "address-data") | (CALDAV, "calendar-data") => match res {
            Res::Item { data: Some(d), .. } => escape(d),
            _ => return None,
        },
        _ => return None,
    })
}

fn allprops() -> Vec<Prop> {
    vec![
        prop(DAV, "resourcetype"),
        prop(DAV, "displayname"),
        prop(DAV, "getetag"),
        prop(DAV, "getcontenttype"),
        prop(CS, "getctag"),
    ]
}

fn response_xml(api: &Api, res: &Res, props: &[Prop], all: bool) -> String {
    let mut found = String::new();
    let mut missing = String::new();
    let asked = if all { allprops() } else { props.to_vec() };
    for p in &asked {
        match value(api, res, p) {
            Some(v) => found.push_str(&element(p, &v)),
            None if !all => missing.push_str(&element(p, "")),
            None => {}
        }
    }
    let mut out = format!("<d:response>{}", href(&res.href()));
    if !found.is_empty() {
        out.push_str(&format!(
            "<d:propstat><d:prop>{found}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat>"
        ));
    }
    if !missing.is_empty() {
        out.push_str(&format!("<d:propstat><d:prop>{missing}</d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>"));
    }
    out.push_str("</d:response>");
    out
}

fn multistatus(responses: &[String]) -> Response {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"{CALDAV}\" xmlns:a=\"{CARDDAV}\" xmlns:cs=\"{CS}\">{}</d:multistatus>",
        responses.concat()
    );
    respond(
        StatusCode::MULTI_STATUS,
        "application/xml; charset=utf-8",
        body,
    )
}

async fn collection(api: &Api, book: Book, id: Hash) -> Result<Res> {
    let (name, seq) = api
        .node
        .with_space(&id, |l| (l.space.name().to_string(), l.space.seq))
        .await?;
    Ok(Res::Collection {
        book,
        id,
        name,
        seq,
    })
}

/// The items of a collection. `with_data` renders each one.
async fn items(api: &Api, book: Book, id: &Hash, with_data: bool) -> Result<Vec<Res>> {
    let rows = api
        .node
        .index
        .list(&id.to_hex(), Some(book.kind()), 100_000, 0)
        .await
        .map_err(hs_node::Error::from)?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let stem = r.target.split_once('/')?.1.to_string();
            let etag = rest::etag(&Hash::from_hex(r.payload.as_deref()?)?);
            let data = if with_data {
                r.body.as_ref().map(|b| book.render(b, &r.target))
            } else {
                None
            };
            Some(Res::Item {
                book,
                id: *id,
                stem,
                etag,
                data,
            })
        })
        .collect())
}

async fn item(
    api: &Api,
    book: Book,
    id: &Hash,
    stem: &str,
    with_data: bool,
) -> Result<Option<Res>> {
    let target = format!("{}/{stem}", book.kind());
    let Some((r, bytes)) = api.node.get(id, &target).await? else {
        return Ok(None);
    };
    let (Some(hash), Some(bytes)) = (r.payload, bytes) else {
        return Ok(None);
    };
    let data = with_data.then(|| {
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        book.render(&v, &target)
    });
    Ok(Some(Res::Item {
        book,
        id: *id,
        stem: stem.to_string(),
        etag: rest::etag(&hash),
        data,
    }))
}

fn stem_of(file: &str, book: Book) -> Option<String> {
    let s = percent_decode(file, false);
    s.strip_suffix(book.ext())
        .map(String::from)
        .filter(|s| !s.is_empty())
}

fn depth(req: &Request) -> u8 {
    match hdr(req, "depth") {
        Some("0") => 0,
        _ => 1,
    }
}

async fn space_of(api: &Api, access: &Access, s: &str) -> Result<Hash> {
    rest::space(api, access, s).await
}

async fn propfind(api: &Api, req: &Request, access: &Access, segs: &[&str]) -> Result {
    let ask = parse_body(req.body());
    let d = depth(req);
    let mut out: Vec<Res> = Vec::new();
    match segs {
        [] => {
            out.push(Res::Root);
            if d > 0 {
                out.push(Res::Home(Book::Contacts));
                out.push(Res::Home(Book::Calendars));
            }
        }
        [b] => {
            let book = Book::parse(b).ok_or_else(ApiError::not_found)?;
            out.push(Res::Home(book));
            if d > 0 {
                for id in rest::readable(api, access) {
                    if rest::type_ok(access, book.kind()) {
                        out.push(collection(api, book, id).await?);
                    }
                }
            }
        }
        [b, s] => {
            let book = Book::parse(b).ok_or_else(ApiError::not_found)?;
            let id = space_of(api, access, s).await?;
            out.push(collection(api, book, id).await?);
            if d > 0 {
                out.extend(items(api, book, &id, false).await?);
            }
        }
        [b, s, f] => {
            let book = Book::parse(b).ok_or_else(ApiError::not_found)?;
            let id = space_of(api, access, s).await?;
            let stem = stem_of(f, book).ok_or_else(ApiError::not_found)?;
            out.push(
                item(api, book, &id, &stem, false)
                    .await?
                    .ok_or_else(ApiError::not_found)?,
            );
        }
        _ => return Err(ApiError::not_found()),
    }
    Ok(multistatus(
        &out.iter()
            .map(|r| response_xml(api, r, &ask.props, ask.all))
            .collect::<Vec<_>>(),
    ))
}

async fn report(api: &Api, req: &Request, access: &Access, segs: &[&str]) -> Result {
    let [b, s, ..] = segs else {
        return Err(ApiError::not_found());
    };
    let book = Book::parse(b).ok_or_else(ApiError::not_found)?;
    let id = space_of(api, access, s).await?;
    let ask = parse_body(req.body());
    let wants_data = ask
        .props
        .iter()
        .any(|(_, n)| n == "address-data" || n == "calendar-data");
    let found = match ask.report.as_deref() {
        Some("addressbook-multiget" | "calendar-multiget") => {
            let mut out = Vec::new();
            let mut missing = Vec::new();
            let seen: BTreeSet<String> = ask.hrefs.iter().cloned().collect();
            for h in seen {
                let file = h.trim_end_matches('/').rsplit('/').next().unwrap_or("");
                match stem_of(file, book) {
                    Some(stem) => match item(api, book, &id, &stem, wants_data).await? {
                        Some(r) => out.push(r),
                        None => missing.push(h),
                    },
                    None => missing.push(h),
                }
            }
            let mut xml: Vec<String> = out
                .iter()
                .map(|r| response_xml(api, r, &ask.props, false))
                .collect();
            for h in missing {
                xml.push(format!(
                    "<d:response>{}<d:status>HTTP/1.1 404 Not Found</d:status></d:response>",
                    href(&h)
                ));
            }
            return Ok(multistatus(&xml));
        }
        Some("addressbook-query" | "calendar-query") => items(api, book, &id, wants_data).await?,
        _ => return Err(ApiError::forbidden("unsupported report")),
    };
    Ok(multistatus(
        &found
            .iter()
            .map(|r| response_xml(api, r, &ask.props, false))
            .collect::<Vec<_>>(),
    ))
}

async fn get(
    api: &Api,
    req: &Request,
    access: &Access,
    book: Book,
    id: &Hash,
    stem: &str,
) -> Result {
    let Some(Res::Item { etag, data, .. }) = item(api, book, id, stem, true).await? else {
        return Err(ApiError::not_found());
    };
    let _ = access;
    if hdr(req, header::IF_NONE_MATCH)
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"))
    {
        let mut r = empty(StatusCode::NOT_MODIFIED);
        set_header(&mut r, header::ETAG, &etag);
        return Ok(r);
    }
    let mut r = respond(StatusCode::OK, book.media_type(), data.unwrap_or_default());
    set_header(&mut r, header::ETAG, &etag);
    Ok(r)
}

async fn put(
    api: &Api,
    req: &Request,
    access: &Access,
    book: Book,
    id: &Hash,
    stem: &str,
) -> Result {
    let text = String::from_utf8_lossy(req.body());
    let v = match book {
        Book::Contacts if text.contains("BEGIN:VCARD") => formats::from_vcard(&text),
        Book::Calendars if text.contains("BEGIN:VCALENDAR") => formats::from_ical(&text),
        _ => {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "expected vCard or iCalendar",
            ));
        }
    };
    let target = format!("{}/{stem}", book.kind());
    let bytes = serde_json::to_vec(&v).unwrap_or_default();
    let r = rest::put(api, req, access, id, &target, bytes, "application/json").await?;
    let mut out = empty(r.status());
    if let Some(tag) = r.headers().get(header::ETAG) {
        out.headers_mut().insert(header::ETAG, tag.clone());
    }
    Ok(out)
}

async fn delete(
    api: &Api,
    req: &Request,
    access: &Access,
    book: Book,
    id: &Hash,
    stem: &str,
) -> Result {
    let target = format!("{}/{stem}", book.kind());
    rest::delete_resource(api, req, access, id, &target).await
}

fn options() -> Response {
    let mut r = empty(StatusCode::OK);
    r.headers_mut().insert(
        "dav",
        HeaderValue::from_static("1, 3, addressbook, calendar-access"),
    );
    r.headers_mut().insert(
        header::ALLOW,
        HeaderValue::from_static("OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, REPORT"),
    );
    r
}

pub async fn handle(api: &Api, req: &Request, access: &Access, segs: &[&str]) -> Result {
    let segs: Vec<&str> = segs.iter().copied().filter(|s| !s.is_empty()).collect();
    if req.method() == Method::OPTIONS {
        return Ok(options());
    }
    if matches!(access, Access::Public) {
        return Err(ApiError::unauthorized());
    }
    match req.method().as_str() {
        "PROPFIND" => return propfind(api, req, access, &segs).await,
        "REPORT" => return report(api, req, access, &segs).await,
        _ => {}
    }
    let [b, s, f] = segs.as_slice() else {
        return Err(ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "not here"));
    };
    let book = Book::parse(b).ok_or_else(ApiError::not_found)?;
    if !rest::type_ok(access, book.kind()) {
        return Err(ApiError::not_found());
    }
    let id = space_of(api, access, s).await?;
    let stem = stem_of(f, book).ok_or_else(ApiError::not_found)?;
    match *req.method() {
        Method::GET | Method::HEAD => get(api, req, access, book, &id, &stem).await,
        Method::PUT => put(api, req, access, book, &id, &stem).await,
        Method::DELETE => delete(api, req, access, book, &id, &stem).await,
        _ => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_propfind() {
        let a = parse_body(
            br#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/"><d:prop><d:getetag/><cs:getctag/></d:prop></d:propfind>"#,
        );
        assert_eq!(a.report.as_deref(), Some("propfind"));
        assert!(!a.all);
        assert_eq!(a.props, vec![prop(DAV, "getetag"), prop(CS, "getctag")]);
        assert!(parse_body(b"").all);
    }

    #[test]
    fn parses_multiget() {
        let a = parse_body(
            br#"<c:calendar-multiget xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data/></d:prop><d:href>/dav/calendars/x/a&amp;b.ics</d:href><d:href>/dav/calendars/x/c.ics</d:href></c:calendar-multiget>"#,
        );
        assert_eq!(a.report.as_deref(), Some("calendar-multiget"));
        assert_eq!(
            a.hrefs,
            vec!["/dav/calendars/x/a&b.ics", "/dav/calendars/x/c.ics"]
        );
        assert_eq!(a.props[1], prop(CALDAV, "calendar-data"));
    }

    #[test]
    fn unknown_namespace_is_declared() {
        assert_eq!(
            element(&prop("urn:x", "foo"), ""),
            "<x:foo xmlns:x=\"urn:x\"/>"
        );
        assert_eq!(
            element(&prop(DAV, "getetag"), "\"1\""),
            "<d:getetag>\"1\"</d:getetag>"
        );
    }
}
