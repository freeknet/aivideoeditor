// ============================================================================
// Module:       search
// Description:  Search over the shot log: the query language (free text +
//               facets like fire>0.3, person:x, flag:dark), scoring with
//               match reasons, and more-like-this.
//
// Dependencies: crate::{index, people, analyze::segments}, self::marks
// ============================================================================

//! Search over the shot log.
//!
//! One query box understands free text and facets together, the way the
//! shot log reads: `poi hoodie fire>0.5 -flag:soft move:static`.
//!
//! ## Tokens
//!
//! | Token | Keeps segments… |
//! |---|---|
//! | `word`, `"a phrase"` | whose caption (summary, people, setting, issues), person name or clip id mentions it |
//! | `fire`, `led`, `light` | with that light source visible |
//! | `dark`, `soft`, `shaky`, `whip`, `blown`, `short`, `no-action`, `flag:<f>` | carrying that flag |
//! | `clean` | with no flags at all |
//! | `fire>0.3`, `led<0.2`, `light>0.5`, `move>0.4` | by share of frame, **relative to the night** (1 = the night's strongest) |
//! | `fire>2%` | by raw share of frame |
//! | `q>0.7`, `focus<0.8`, `exp>0.5`, `luma<0.1`, `dur>3`, `cx<0.4`, `rating>=3` | by quality, focus (1 = the night's median), highlights (p95), mean luma, seconds, action centre, review rating |
//! | `move:pan`, `move:static`, `move:zoom-in`, `move:zoom-out`, `move:shaky`, `move:whip`, `static`, `pan` | by camera behaviour |
//! | `person:<name>` | whose people-map name (or caption description) contains `<name>` |
//! | `clip:MVI_79` | of clips whose id starts with that (or that exact segment id) |
//! | `pick`, `reject`, `unreviewed`, `rated` | by review mark (see [`marks`]) |
//! | `moment:performing`, `framing:close`, `setting:stage` | by caption field |
//! | `captioned`, `uncaptioned` | by whether the vision model has seen it |
//! | `like:MVI_7925/3` | ranked by similarity to that segment (more like this) |
//!
//! A leading `-` negates any token (`-dark`, `-person:white-mask`).
//! Comparators are `>`, `>=`, `<`, `<=`, `=`.
//!
//! ## Ranking
//!
//! Facets filter; free text and `like:` rank. Every hit carries a score in
//! 0..1 and a one-line reason saying why it is there ("caption: …poi
//! spinner…; fire 0.62"), so a low-confidence hit is visible as one and the
//! interface can cut the tail off.
//!
//! ## Completeness
//!
//! Free text and the caption facets only see captioned segments. The
//! [`Results`] say so: `uncaptioned_in_scope` counts the segments that pass
//! every measured filter but have no caption, and `complete` is false when
//! the query reads captions and that count is not zero, so the Shots
//! browser and the chat agent know the hits are partial.

pub(crate) mod marks;

use std::collections::BTreeSet;

use crate::analyze::segments::percentile;
use crate::index::{Index, Scored};
use crate::people::People;

use self::marks::{Marks, Verdict};

/// A comparison in a numeric facet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cmp {
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `=` (within 0.005)
    Eq,
}

impl Cmp {
    /// Whether `x` passes against `v`.
    fn test(self, x: f32, v: f32) -> bool {
        match self {
            Self::Gt => x > v,
            Self::Ge => x >= v,
            Self::Lt => x < v,
            Self::Le => x <= v,
            Self::Eq => (x - v).abs() < 0.005,
        }
    }

    /// The operator as typed.
    fn symbol(self) -> &'static str {
        match self {
            Self::Gt => ">",
            Self::Ge => "≥",
            Self::Lt => "<",
            Self::Le => "≤",
            Self::Eq => "=",
        }
    }
}

/// A number a facet can compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Metric {
    /// Flame share, night-relative.
    Fire,
    /// LED share, night-relative.
    Led,
    /// White-light share, night-relative.
    Light,
    /// Moving-light share, night-relative.
    Move,
    /// Quality 0..1.
    Q,
    /// Focus relative to the night's median.
    Focus,
    /// 95th-percentile luma.
    Exp,
    /// Mean luma.
    Luma,
    /// Length, seconds.
    Dur,
    /// Action centre x.
    Cx,
    /// Review rating 0..5.
    Rating,
}

impl Metric {
    /// The metric a facet name stands for.
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "fire" | "flame" => Self::Fire,
            "led" => Self::Led,
            "light" | "white" => Self::Light,
            "move" | "motion" | "moving" | "action" => Self::Move,
            "q" | "quality" | "score" => Self::Q,
            "focus" | "sharp" | "sharpness" => Self::Focus,
            "exp" | "exposure" | "p95" | "highlights" => Self::Exp,
            "luma" | "bright" | "brightness" => Self::Luma,
            "dur" | "len" | "length" | "duration" => Self::Dur,
            "cx" | "x" => Self::Cx,
            "rating" | "stars" => Self::Rating,
            _ => return None,
        })
    }

    /// The name a chip and a reason show.
    fn label(self) -> &'static str {
        match self {
            Self::Fire => "fire",
            Self::Led => "led",
            Self::Light => "light",
            Self::Move => "move",
            Self::Q => "q",
            Self::Focus => "focus",
            Self::Exp => "exp",
            Self::Luma => "luma",
            Self::Dur => "dur",
            Self::Cx => "cx",
            Self::Rating => "rating",
        }
    }

    /// Whether the metric is a share of frame (and so night-relative).
    fn is_share(self) -> bool {
        matches!(self, Self::Fire | Self::Led | Self::Light | Self::Move)
    }
}

/// What a facet asks for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Term {
    /// A word or phrase matched against captions, names and ids.
    Text(String),
    /// A number compared: `fire>0.3`; `raw` when typed as a percentage.
    Metric {
        /// Which number.
        metric: Metric,
        /// How.
        cmp: Cmp,
        /// Against what.
        value: f32,
        /// The value is a raw share of frame (`fire>2%`), not night-relative.
        raw: bool,
    },
    /// That light source is visible: `fire`, `led`, `light`.
    Source(Metric),
    /// Carries a flag.
    Flag(String),
    /// Carries no flags.
    Clean,
    /// Camera behaviour, by its shot-log label.
    Camera(String),
    /// People-map name or caption description.
    Person(String),
    /// Clip id prefix or segment id.
    Clip(String),
    /// Picked.
    Pick,
    /// Rejected.
    Reject,
    /// Neither picked nor rejected.
    Unreviewed,
    /// Has a star rating.
    Rated,
    /// Caption moment.
    Moment(String),
    /// Caption framing.
    Framing(String),
    /// Caption setting.
    Setting(String),
    /// Has a caption.
    Captioned,
    /// Has no caption yet.
    Uncaptioned,
    /// More like this segment.
    Like(String),
}

/// One token of a query: what it asks for, whether it is negated, and the
/// text it was parsed from (so a chip can be removed from the query).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Clause {
    /// The token as typed.
    pub(crate) raw: String,
    /// A leading `-`.
    pub(crate) negate: bool,
    /// What it asks for.
    pub(crate) term: Term,
}

impl Clause {
    /// The label a chip shows.
    pub(crate) fn label(&self) -> String {
        let body = match &self.term {
            Term::Text(t) if t.contains(' ') => format!("“{t}”"),
            Term::Text(t) => t.clone(),
            Term::Metric {
                metric,
                cmp,
                value,
                raw,
            } => {
                if *raw {
                    format!("{} {} {:.1}%", metric.label(), cmp.symbol(), value * 100.0)
                } else {
                    format!("{} {} {}", metric.label(), cmp.symbol(), trim_num(*value))
                }
            }
            Term::Source(m) => m.label().to_owned(),
            Term::Flag(f) => format!("flag {f}"),
            Term::Clean => "no flags".to_owned(),
            Term::Camera(c) => format!("move {c}"),
            Term::Person(p) => format!("person {p}"),
            Term::Clip(c) => format!("clip {c}"),
            Term::Pick => "picked".to_owned(),
            Term::Reject => "rejected".to_owned(),
            Term::Unreviewed => "unreviewed".to_owned(),
            Term::Rated => "rated".to_owned(),
            Term::Moment(m) => format!("moment {m}"),
            Term::Framing(f) => format!("framing {f}"),
            Term::Setting(s) => format!("setting {s}"),
            Term::Captioned => "captioned".to_owned(),
            Term::Uncaptioned => "uncaptioned".to_owned(),
            Term::Like(id) => format!("like {id}"),
        };
        if self.negate {
            format!("not {body}")
        } else {
            body
        }
    }

    /// Whether this is free text (shown plainly, not as a facet chip).
    pub(crate) fn is_text(&self) -> bool {
        matches!(self.term, Term::Text(_))
    }
}

/// A number without trailing zeros: `0.3`, `2`, `0.75`.
fn trim_num(v: f32) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_owned()
}

/// A parsed query.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Query {
    /// The tokens, in typed order.
    pub(crate) clauses: Vec<Clause>,
}

/// Flags the indexer sets, accepted bare and after `flag:`.
const FLAGS: [&str; 7] = [
    "dark",
    "soft",
    "shaky",
    "whip",
    "blown",
    "no-action",
    "short",
];

/// Camera labels accepted after `move:`.
const CAMERAS: [&str; 7] = [
    "static", "pan", "zoom-in", "zoom-out", "whip", "shaky", "unknown",
];

impl Query {
    /// Parses a query. Never fails: anything not understood as a facet is
    /// free text.
    pub(crate) fn parse(text: &str) -> Self {
        Self {
            clauses: tokens(text).into_iter().filter_map(clause).collect(),
        }
    }

    /// Whether there is nothing to filter or rank by.
    pub(crate) fn is_empty(&self) -> bool {
        self.clauses.is_empty()
    }

    /// Whether the query ranks (free text or `like:`), so "best match" is
    /// a meaningful order.
    pub(crate) fn ranks(&self) -> bool {
        self.clauses
            .iter()
            .any(|c| !c.negate && matches!(c.term, Term::Text(_) | Term::Like(_)))
    }

    /// The query text with clause `i` removed.
    pub(crate) fn without(&self, i: usize) -> String {
        self.clauses
            .iter()
            .enumerate()
            .filter(|(k, _)| *k != i)
            .map(|(_, c)| c.raw.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Splits on whitespace, keeping `"quoted phrases"` (and `-"negated
/// phrases"`) together.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in text.chars() {
        if ch == '"' {
            cur.push(ch);
            quoted = !quoted;
            if !quoted {
                out.push(std::mem::take(&mut cur));
            }
        } else if ch.is_whitespace() && !quoted {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// One token to a clause.
fn clause(raw: String) -> Option<Clause> {
    let (negate, body) = match raw.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        Some(_) | None => (false, raw.as_str()),
    };
    let term = term(body)?;
    Some(Clause { raw, negate, term })
}

/// A token body (no `-`) to a term.
fn term(body: &str) -> Option<Term> {
    if let Some(phrase) = body.strip_prefix('"') {
        let phrase = phrase.trim_end_matches('"').trim().to_lowercase();
        return (!phrase.is_empty()).then_some(Term::Text(phrase));
    }
    let lower = body.to_lowercase();
    if let Some(t) = comparison(&lower) {
        return Some(t);
    }
    if let Some((key, value)) = lower.split_once(':') {
        let value = value.trim().to_owned();
        if value.is_empty() {
            return None;
        }
        let keep_case = || {
            body.split_once(':')
                .map_or(value.clone(), |(_, v)| v.trim().to_owned())
        };
        let t = match key {
            "flag" | "is" if FLAGS.contains(&value.as_str()) => Term::Flag(value),
            "flag" | "is" if value == "none" || value == "clean" => Term::Clean,
            "move" | "camera" | "cam" => Term::Camera(camera_alias(&value)),
            "person" | "who" | "p" => Term::Person(value),
            "clip" | "seg" | "id" => Term::Clip(value),
            "moment" => Term::Moment(value),
            "framing" | "frame" | "shot" => Term::Framing(value),
            "setting" | "where" => Term::Setting(value),
            "like" | "similar" => Term::Like(keep_case()),
            "subject" | "is" => match Metric::named(&value) {
                Some(m @ (Metric::Fire | Metric::Led | Metric::Light)) => Term::Source(m),
                Some(_) | None => Term::Text(lower),
            },
            "review" | "mark" => match value.as_str() {
                "pick" | "picked" => Term::Pick,
                "reject" | "rejected" => Term::Reject,
                "none" | "unreviewed" => Term::Unreviewed,
                _ => Term::Text(lower),
            },
            _ => Term::Text(lower),
        };
        return Some(t);
    }
    Some(match lower.as_str() {
        "fire" | "flame" | "flames" => Term::Source(Metric::Fire),
        "led" | "leds" => Term::Source(Metric::Led),
        "light" => Term::Source(Metric::Light),
        "clean" => Term::Clean,
        "pick" | "picked" | "picks" => Term::Pick,
        "reject" | "rejected" | "rejects" => Term::Reject,
        "unreviewed" => Term::Unreviewed,
        "rated" => Term::Rated,
        "captioned" => Term::Captioned,
        "uncaptioned" => Term::Uncaptioned,
        "static" | "pan" | "zoom-in" | "zoom-out" => Term::Camera(lower),
        f if FLAGS.contains(&f) => Term::Flag(lower),
        _ => Term::Text(lower),
    })
}

/// Accepts `zoom`, `zoomin`, `in`, `handheld` as camera names.
fn camera_alias(value: &str) -> String {
    match value {
        "zoom" | "zoomin" | "in" | "push" => "zoom-in",
        "zoomout" | "out" | "pull" => "zoom-out",
        "handheld" | "shake" => "shaky",
        "locked" | "still" | "tripod" => "static",
        "whip-pan" => "whip",
        v if CAMERAS.contains(&v) => v,
        v => return v.to_owned(),
    }
    .to_owned()
}

/// `name<op>value` to a metric term.
fn comparison(lower: &str) -> Option<Term> {
    let pos = lower.find(['>', '<', '='])?;
    let (name, rest) = lower.split_at(pos);
    let metric = Metric::named(name)?;
    let (cmp, value) = if let Some(v) = rest.strip_prefix(">=") {
        (Cmp::Ge, v)
    } else if let Some(v) = rest.strip_prefix("<=") {
        (Cmp::Le, v)
    } else if let Some(v) = rest.strip_prefix('>') {
        (Cmp::Gt, v)
    } else if let Some(v) = rest.strip_prefix('<') {
        (Cmp::Lt, v)
    } else {
        (Cmp::Eq, rest.trim_start_matches('='))
    };
    let (value, raw) = match value.strip_suffix('%') {
        Some(pct) => (pct.parse::<f32>().ok()? / 100.0, metric.is_share()),
        None => (value.parse::<f32>().ok()?, false),
    };
    value.is_finite().then_some(Term::Metric {
        metric,
        cmp,
        value,
        raw,
    })
}

/// Night-wide scales: what "1.0" means for each share of frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scales {
    fire: f32,
    led: f32,
    light: f32,
    moving: f32,
    luma: f32,
    sharp_ref: f32,
}

impl Scales {
    /// The 98th percentile of each share over the night, so one freak
    /// segment does not squash everything else towards zero.
    fn of(index: &Index) -> Self {
        let p = |f: fn(&Scored) -> f32| {
            let v: Vec<f32> = index.segments.iter().map(f).collect();
            percentile(&v, 0.98).max(1e-4)
        };
        Self {
            fire: p(|s| s.seg.flame),
            led: p(|s| s.seg.led),
            light: p(|s| s.seg.white),
            moving: p(|s| s.seg.moving),
            luma: p(|s| s.seg.luma),
            sharp_ref: index.sharp_ref.max(1e-4),
        }
    }
}

/// What a search runs against.
pub(crate) struct Night<'a> {
    /// The shot log.
    pub(crate) index: &'a Index,
    /// Who is in which clip.
    pub(crate) people: &'a People,
    /// Review marks.
    pub(crate) marks: &'a Marks,
    scales: Scales,
}

impl<'a> Night<'a> {
    /// Prepares a search over `index`.
    pub(crate) fn new(index: &'a Index, people: &'a People, marks: &'a Marks) -> Self {
        Self {
            index,
            people,
            marks,
            scales: Scales::of(index),
        }
    }

    /// A metric of a segment, on the scale facets compare.
    pub(crate) fn metric(&self, s: &Scored, metric: Metric) -> f32 {
        let sc = &self.scales;
        match metric {
            Metric::Fire => (s.seg.flame / sc.fire).min(1.0),
            Metric::Led => (s.seg.led / sc.led).min(1.0),
            Metric::Light => (s.seg.white / sc.light).min(1.0),
            Metric::Move => (s.seg.moving / sc.moving).min(1.0),
            Metric::Q => s.q,
            Metric::Focus => s.seg.sharp / sc.sharp_ref,
            Metric::Exp => s.seg.p95,
            Metric::Luma => s.seg.luma,
            Metric::Dur => s.seg.end - s.seg.start,
            Metric::Cx => s.seg.cx,
            Metric::Rating => f32::from(self.marks.get(&s.id).rating),
        }
    }

    /// The raw share of frame for a share metric.
    fn raw(s: &Scored, metric: Metric) -> f32 {
        match metric {
            Metric::Fire => s.seg.flame,
            Metric::Led => s.seg.led,
            Metric::Light => s.seg.white,
            Metric::Move => s.seg.moving,
            Metric::Q
            | Metric::Focus
            | Metric::Exp
            | Metric::Luma
            | Metric::Dur
            | Metric::Cx
            | Metric::Rating => 0.0,
        }
    }

    /// The people-map name for a segment.
    pub(crate) fn person(&self, s: &Scored) -> String {
        self.people.name_at(&s.clip, s.seg.start)
    }
}

/// One result.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    /// Index into `Index::segments`.
    pub(crate) seg: usize,
    /// Overall score 0..1: relevance blended with quality.
    pub(crate) score: f32,
    /// Why it matched, one line.
    pub(crate) reason: String,
}

/// What a search found, and how much of the night it could see.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Results {
    /// The hits, best first.
    pub(crate) hits: Vec<Hit>,
    /// False when the query reads captions (free text, `person:`,
    /// `moment:`, `framing:`, `setting:`) and some segments it would
    /// otherwise have considered have no caption: the results are partial.
    pub(crate) complete: bool,
    /// Segments that pass the query's non-caption filters but have no
    /// caption, so a caption query could not see them.
    pub(crate) uncaptioned_in_scope: usize,
}

/// Whether a positive clause can only match through a caption, so an
/// uncaptioned segment is invisible to it.
fn reads_captions(c: &Clause) -> bool {
    !c.negate
        && match c.term {
            Term::Text(_)
            | Term::Person(_)
            | Term::Moment(_)
            | Term::Framing(_)
            | Term::Setting(_) => true,
            Term::Metric { .. }
            | Term::Source(_)
            | Term::Flag(_)
            | Term::Clean
            | Term::Camera(_)
            | Term::Clip(_)
            | Term::Pick
            | Term::Reject
            | Term::Unreviewed
            | Term::Rated
            | Term::Captioned
            | Term::Uncaptioned
            | Term::Like(_) => false,
        }
}

/// Below this a source is "not visible" (matches the indexer's subject test).
const SOURCE_MIN: f32 = 0.003;

/// Runs `query` over the night. Hits come best first; the results say how
/// many uncaptioned segments a caption query could not see.
pub(crate) fn search(query: &Query, night: &Night<'_>) -> Results {
    let like = query
        .clauses
        .iter()
        .find_map(|c| match (&c.term, c.negate) {
            (Term::Like(id), false) => night
                .index
                .segments
                .iter()
                .position(|s| s.id.eq_ignore_ascii_case(id)),
            _ => None,
        });
    let words: Vec<&str> = query
        .clauses
        .iter()
        .filter(|c| !c.negate)
        .filter_map(|c| {
            if let Term::Text(t) = &c.term {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect();
    let mut hits = Vec::new();
    let caption_bound = query.clauses.iter().any(reads_captions);
    let mut uncaptioned_in_scope = 0;
    for (i, s) in night.index.segments.iter().enumerate() {
        if like == Some(i) {
            continue;
        }
        if s.caption.is_none()
            && query
                .clauses
                .iter()
                .filter(|c| !reads_captions(c) && !matches!(c.term, Term::Like(_)))
                .all(|c| facet(night, s, &c.term).0 != c.negate)
        {
            uncaptioned_in_scope += 1;
        }
        let mut reasons: Vec<String> = Vec::new();
        let mut pass = true;
        for c in &query.clauses {
            if matches!(c.term, Term::Like(_)) || (!c.negate && c.is_text()) {
                continue;
            }
            let (ok, why) = facet(night, s, &c.term);
            if ok == c.negate {
                pass = false;
                break;
            }
            if !c.negate
                && let Some(w) = why
            {
                reasons.push(w);
            }
        }
        if !pass {
            continue;
        }
        let mut relevance = None;
        if !words.is_empty() {
            let (r, why) = text_relevance(night, s, &words);
            if r <= 0.0 {
                continue;
            }
            relevance = Some(r);
            reasons.splice(0..0, why);
        }
        if let Some(l) = like.and_then(|k| night.index.segments.get(k)) {
            let (sim, why) = similarity(night, l, s);
            relevance = Some(relevance.map_or(sim, |r| (r + sim) / 2.0));
            reasons.insert(0, why);
        }
        let score = match relevance {
            Some(r) => 0.75 * r + 0.25 * s.q,
            None => s.q,
        };
        hits.push(Hit {
            seg: i,
            score: score.clamp(0.0, 1.0),
            reason: reasons.join("; "),
        });
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.seg.cmp(&b.seg)));
    Results {
        hits,
        complete: !caption_bound || uncaptioned_in_scope == 0,
        uncaptioned_in_scope,
    }
}

/// Whether a segment passes a (non-text) facet, and the reason to show.
fn facet(night: &Night<'_>, s: &Scored, term: &Term) -> (bool, Option<String>) {
    let caption = s.caption.as_ref();
    let has = |needle: &str, hay: &str| hay.to_lowercase().contains(needle);
    match term {
        Term::Text(t) => {
            let (r, _) = text_relevance(night, s, &[t.as_str()]);
            (r > 0.0, None)
        }
        Term::Metric {
            metric,
            cmp,
            value,
            raw,
        } => {
            if *raw {
                let x = Night::raw(s, *metric);
                (
                    cmp.test(x, *value),
                    Some(format!("{} {:.1}%", metric.label(), x * 100.0)),
                )
            } else {
                let x = night.metric(s, *metric);
                (
                    cmp.test(x, *value),
                    Some(format!("{} {}", metric.label(), fmt_metric(*metric, x))),
                )
            }
        }
        Term::Source(m) => {
            let x = Night::raw(s, *m);
            (
                x > SOURCE_MIN,
                Some(format!("{} {:.2}", m.label(), night.metric(s, *m))),
            )
        }
        Term::Flag(f) => (s.flags.iter().any(|g| g == f), Some(format!("flag {f}"))),
        Term::Clean => (s.flags.is_empty(), None),
        Term::Camera(c) => (s.seg.camera.label() == c, Some(format!("camera {c}"))),
        Term::Person(p) => {
            let name = night.person(s);
            if has(p, &name) && name != s.clip {
                return (true, Some(format!("person {name}")));
            }
            let described = caption.and_then(|c| {
                c.people
                    .iter()
                    .find(|q| has(p, &q.appearance) || has(p, &q.prop))
                    .map(|q| q.appearance.clone())
            });
            match described {
                Some(d) => (true, Some(format!("who: {}", clip_text(&d, 40)))),
                None => (false, None),
            }
        }
        Term::Clip(c) => {
            let id = s.id.to_lowercase();
            let ok = if c.contains('/') {
                id == *c
            } else {
                s.clip.to_lowercase().starts_with(c.as_str())
            };
            (ok, None)
        }
        Term::Pick => (
            night.marks.get(&s.id).verdict == Some(Verdict::Pick),
            Some("picked".to_owned()),
        ),
        Term::Reject => (
            night.marks.get(&s.id).verdict == Some(Verdict::Reject),
            Some("rejected".to_owned()),
        ),
        Term::Unreviewed => (night.marks.get(&s.id).verdict.is_none(), None),
        Term::Rated => {
            let r = night.marks.get(&s.id).rating;
            (r > 0, Some(format!("{r}★")))
        }
        Term::Moment(m) => match caption {
            Some(c) => (
                c.effective_moment().to_lowercase().starts_with(m.as_str()),
                Some(format!("moment {}", c.effective_moment())),
            ),
            None => (false, None),
        },
        Term::Framing(f) => match caption {
            Some(c) => (
                c.framing.to_lowercase().starts_with(f.as_str()),
                Some(format!("framing {}", c.framing)),
            ),
            None => (false, None),
        },
        Term::Setting(w) => match caption {
            Some(c) => (has(w, &c.setting), Some(format!("setting {}", c.setting))),
            None => (false, None),
        },
        Term::Captioned => (caption.is_some(), None),
        Term::Uncaptioned => (caption.is_none(), Some("no caption yet".to_owned())),
        Term::Like(_) => (true, None),
    }
}

/// A metric value as a reason shows it.
fn fmt_metric(metric: Metric, x: f32) -> String {
    match metric {
        Metric::Dur => format!("{x:.1}s"),
        Metric::Rating => format!("{x:.0}★"),
        Metric::Fire
        | Metric::Led
        | Metric::Light
        | Metric::Move
        | Metric::Q
        | Metric::Focus
        | Metric::Exp
        | Metric::Luma
        | Metric::Cx => format!("{x:.2}"),
    }
}

/// Words a query word also matches.
fn synonyms(word: &str) -> &'static [&'static str] {
    match word {
        "fire" | "flame" | "flames" | "fiery" => &["fire", "flame", "flaming", "burning"],
        "crowd" | "audience" => &["crowd", "audience", "spectators", "onlookers"],
        "dj" | "decks" => &["dj", "decks", "turntable", "mixer", "booth"],
        "silks" | "aerial" => &["silk", "aerial", "fabric", "hanging"],
        "spin" | "spinning" | "spinner" => &["spin", "twirl", "swing"],
        "glow" | "glowing" => &["glow", "lit", "illuminat"],
        "dance" | "dancing" | "dancer" => &["danc"],
        "night" | "dark" => &["night", "dark"],
        _ => &[],
    }
}

/// Lowercase alphanumeric words of `text`.
fn words_of(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

/// Whether query word `q` matches the text `hay`: a word of it starts with
/// `q` (or a synonym), or, for phrases, the text contains it.
fn word_hits(q: &str, hay: &str) -> bool {
    if q.contains(' ') {
        return hay.to_lowercase().contains(q);
    }
    let alts = synonyms(q);
    words_of(hay).any(|w| {
        let starts = |p: &str| {
            if p.len() >= 3 {
                w.starts_with(p)
            } else {
                w == p
            }
        };
        starts(q) || alts.iter().any(|a| starts(a))
    })
}

/// How well free-text words match a segment (0..1) and the reasons.
fn text_relevance(night: &Night<'_>, s: &Scored, words: &[&str]) -> (f32, Vec<String>) {
    let caption = s.caption.as_ref();
    let person = night.person(s);
    let people_text: String = caption
        .map(|c| {
            c.people
                .iter()
                .map(|p| format!("{} {}", p.appearance, p.prop))
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    // (field text, weight, reason label)
    let fields: [(&str, f32, &str); 7] = [
        (caption.map_or("", |c| c.summary.as_str()), 1.0, "caption"),
        (&people_text, 0.9, "who"),
        (if person == s.clip { "" } else { &person }, 1.0, "person"),
        (&s.id, 1.0, "clip"),
        (caption.map_or("", |c| c.setting.as_str()), 0.6, "setting"),
        (
            &caption.map(|c| c.issues.join(", ")).unwrap_or_default(),
            0.6,
            "issues",
        ),
        (
            &caption
                .map(|c| format!("{} {} {}", c.effective_moment(), c.framing, s.subject))
                .unwrap_or_else(|| s.subject.clone()),
            0.5,
            "tags",
        ),
    ];
    let mut total = 0.0;
    let mut reasons: Vec<String> = Vec::new();
    let mut shown: BTreeSet<&str> = BTreeSet::new();
    for q in words {
        let best = fields
            .iter()
            .filter(|(text, _, _)| !text.is_empty() && word_hits(q, text))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((text, weight, label)) = best {
            total += weight;
            if shown.insert(label) {
                reasons.push(match *label {
                    "caption" => format!("caption: “{}”", snippet(text, q)),
                    "clip" | "person" | "tags" => format!("{label}: {}", clip_text(text, 40)),
                    _ => format!("{label}: {}", snippet(text, q)),
                });
            }
        }
    }
    let n = words.len() as f32;
    let frac = total / n;
    // A partial match ranks well below a full one: 1 of 2 words is 0.25.
    let matched = words
        .iter()
        .filter(|q| {
            fields
                .iter()
                .any(|(t, _, _)| !t.is_empty() && word_hits(q, t))
        })
        .count() as f32;
    let coverage = matched / n;
    (frac * coverage, reasons)
}

/// Up to 48 characters of `text` around the first match of `q`.
fn snippet(text: &str, q: &str) -> String {
    let lower = text.to_lowercase();
    let first = q.split(' ').next().unwrap_or(q);
    let at = lower
        .find(first)
        .or_else(|| synonyms(first).iter().find_map(|a| lower.find(a)))
        .unwrap_or(0);
    let chars: Vec<char> = text.chars().collect();
    let at_char = text[..at.min(text.len())].chars().count();
    let from = at_char.saturating_sub(18);
    let to = (from + 48).min(chars.len());
    let mut s: String = chars[from..to].iter().collect();
    if from > 0 {
        s.insert(0, '…');
    }
    if to < chars.len() {
        s.push('…');
    }
    s
}

/// `text` cut to `n` characters with an ellipsis.
fn clip_text(text: &str, n: usize) -> String {
    if text.chars().count() <= n {
        text.to_owned()
    } else {
        let mut s: String = text.chars().take(n).collect();
        s.push('…');
        s
    }
}

/// Words that say nothing about what a shot is.
const STOP: [&str; 40] = [
    "a",
    "an",
    "the",
    "and",
    "with",
    "in",
    "on",
    "of",
    "at",
    "is",
    "are",
    "to",
    "for",
    "by",
    "from",
    "while",
    "into",
    "near",
    "their",
    "his",
    "her",
    "its",
    "person",
    "people",
    "man",
    "woman",
    "performer",
    "performs",
    "performing",
    "wearing",
    "visible",
    "frame",
    "scene",
    "background",
    "foreground",
    "outdoor",
    "night",
    "venue",
    "dark",
    "none",
];

/// The content words of a caption, for overlap.
fn caption_words(s: &Scored) -> BTreeSet<String> {
    let Some(c) = &s.caption else {
        return BTreeSet::new();
    };
    let mut text = c.summary.clone();
    for p in &c.people {
        text.push(' ');
        text.push_str(&p.appearance);
        text.push(' ');
        text.push_str(&p.prop);
    }
    text.push(' ');
    text.push_str(&c.setting);
    words_of(&text)
        .filter(|w| w.len() > 2 && !STOP.contains(&w.as_str()))
        .map(|w| w.trim_end_matches('s').to_owned())
        .collect()
}

/// How alike two segments are, 0..1, with a reason: measured light, motion,
/// exposure, framing and camera, plus shared caption words.
pub(crate) fn similarity(night: &Night<'_>, a: &Scored, b: &Scored) -> (f32, String) {
    let sc = &night.scales;
    let feat = |s: &Scored| -> [f32; 7] {
        [
            (s.seg.flame / sc.fire).min(1.5).sqrt(),
            (s.seg.led / sc.led).min(1.5).sqrt(),
            (s.seg.white / sc.light).min(1.5).sqrt(),
            (s.seg.moving / sc.moving).min(1.5).sqrt(),
            (s.seg.luma / sc.luma).min(1.5),
            s.seg.cx,
            (s.seg.sharp / sc.sharp_ref).min(1.6) / 1.6,
        ]
    };
    const WEIGHTS: [f32; 7] = [1.4, 1.4, 0.6, 1.0, 0.8, 0.5, 0.5];
    let (fa, fb) = (feat(a), feat(b));
    let d2: f32 = fa
        .iter()
        .zip(fb.iter())
        .zip(WEIGHTS.iter())
        .map(|((x, y), w)| w * (x - y) * (x - y))
        .sum();
    let same_cam = a.seg.camera == b.seg.camera;
    let metric = (-d2 * 2.5).exp() * if same_cam { 1.0 } else { 0.85 };
    let (wa, wb) = (caption_words(a), caption_words(b));
    let shared: Vec<&String> = wa.intersection(&wb).collect();
    let union = wa.union(&wb).count();
    let mut parts = Vec::new();
    let sim = if union > 0 && !wa.is_empty() && !wb.is_empty() {
        let jac = shared.len() as f32 / union as f32;
        // Jaccard of short captions rarely passes 0.5; stretch it.
        let words = (jac * 2.0).min(1.0);
        0.55 * metric + 0.45 * words
    } else {
        metric
    };
    if a.subject == b.subject && a.subject != "none" {
        parts.push(a.subject.clone());
    }
    if same_cam {
        parts.push(a.seg.camera.label().to_owned());
    }
    if !shared.is_empty() {
        let w: Vec<&str> = shared.iter().take(4).map(|s| s.as_str()).collect();
        parts.push(format!("shares “{}”", w.join(", ")));
    }
    if a.clip == b.clip {
        parts.push("same clip".to_owned());
    }
    let reason = format!(
        "like {} {:.0}%{}{}",
        a.id,
        sim * 100.0,
        if parts.is_empty() { "" } else { ": " },
        parts.join(", ")
    );
    (sim.clamp(0.0, 1.0), reason)
}

#[cfg(test)]
mod tests {
    use super::marks::{Marks, Verdict};
    use super::{Cmp, Metric, Night, Query, Term, search};
    use crate::analyze::segments::{Camera, Segment};
    use crate::caption::{Caption, Person};
    use crate::index::{Index, Scored};
    use crate::people::People;

    fn seg(flame: f32, led: f32, moving: f32, camera: Camera) -> Segment {
        Segment {
            start: 0.0,
            end: 3.0,
            camera,
            after_cut: false,
            luma: 0.1,
            p95: 0.5,
            clipped: 0.0,
            sharp: 2.0,
            flame,
            led,
            white: 0.01,
            moving,
            cx: 0.5,
            cx_spread: 0.1,
            cy: 0.5,
            zoom_factor: 1.0,
            pan: 0.0,
            jitter: 0.0,
            ..Segment::default()
        }
    }

    fn scored(id: &str, seg: Segment, q: f32, flags: &[&str], summary: Option<&str>) -> Scored {
        Scored {
            id: id.to_owned(),
            clip: id.split('/').next().unwrap_or(id).to_owned(),
            seg,
            subject: "fire".to_owned(),
            q,
            flags: flags.iter().map(|f| (*f).to_owned()).collect(),
            caption: summary.map(|s| Caption {
                summary: s.to_owned(),
                people: vec![Person {
                    appearance: "person in green hoodie".to_owned(),
                    prop: "LED poi".to_owned(),
                }],
                moment: "performing".to_owned(),
                props_active: true,
                blocked: false,
                framing: "medium".to_owned(),
                setting: "outdoor stage".to_owned(),
                issues: Vec::new(),
            }),
            ..Scored::default()
        }
    }

    fn night() -> Index {
        Index {
            source: String::new(),
            sharp_ref: 2.0,
            clips: Vec::new(),
            thresholds: Default::default(),
            skipped: Vec::new(),
            work: None,
            segments: vec![
                scored(
                    "MVI_7901/1",
                    seg(0.05, 0.0, 0.02, Camera::Static),
                    0.8,
                    &[],
                    Some("Fire staff spinner silhouetted against the crowd."),
                ),
                scored(
                    "MVI_7901/2",
                    seg(0.045, 0.0, 0.02, Camera::Static),
                    0.6,
                    &["soft"],
                    Some("Fire staff spinner close to the crowd."),
                ),
                scored(
                    "MVI_7925/1",
                    seg(0.0, 0.08, 0.03, Camera::Pan),
                    0.7,
                    &[],
                    Some("Person performs with glowing LED poi."),
                ),
                scored(
                    "MVI_7930/1",
                    seg(0.0, 0.0, 0.0001, Camera::Shaky),
                    0.1,
                    &["dark", "shaky", "no-action"],
                    None,
                ),
            ],
        }
    }

    fn ids(index: &Index, query: &str, marks: &Marks) -> Vec<String> {
        let people = People::default();
        let n = Night::new(index, &people, marks);
        search(&Query::parse(query), &n)
            .hits
            .into_iter()
            .filter_map(|h| index.segments.get(h.seg).map(|s| s.id.clone()))
            .collect()
    }

    #[test]
    fn facets_and_free_text_are_told_apart() {
        let q = Query::parse(
            r#"fire>0.3 -flag:soft "staff spinner" poi move:pan person:ava q>=0.5 fire>2%"#,
        );
        let terms: Vec<&Term> = q.clauses.iter().map(|c| &c.term).collect();
        assert!(
            matches!(
                terms.first(),
                Some(Term::Metric {
                    metric: Metric::Fire,
                    cmp: Cmp::Gt,
                    raw: false,
                    ..
                })
            ),
            "fire>0.3 is a night-relative metric: {terms:?}"
        );
        assert!(
            q.clauses
                .get(1)
                .is_some_and(|c| c.negate && c.term == Term::Flag("soft".into())),
            "negated flag: {terms:?}"
        );
        assert_eq!(
            terms.get(2),
            Some(&&Term::Text("staff spinner".into())),
            "phrase"
        );
        assert_eq!(terms.get(3), Some(&&Term::Text("poi".into())), "word");
        assert_eq!(terms.get(4), Some(&&Term::Camera("pan".into())), "camera");
        assert_eq!(terms.get(5), Some(&&Term::Person("ava".into())), "person");
        assert!(
            matches!(terms.get(7), Some(Term::Metric { raw: true, .. })),
            "a percentage is a raw share: {terms:?}"
        );
        assert_eq!(
            q.without(2),
            r#"fire>0.3 -flag:soft poi move:pan person:ava q>=0.5 fire>2%"#,
            "a chip removes its token"
        );
    }

    #[test]
    fn bare_facet_words_filter() -> anyhow::Result<()> {
        let index = night();
        let marks = Marks::default();
        assert_eq!(
            ids(&index, "dark", &marks),
            vec!["MVI_7930/1"],
            "bare flag word"
        );
        let fire = ids(&index, "fire", &marks);
        assert_eq!(fire.len(), 2, "fire visible: {fire:?}");
        assert_eq!(
            fire.first().map(String::as_str),
            Some("MVI_7901/1"),
            "best quality first"
        );
        assert_eq!(ids(&index, "led -soft", &marks), vec!["MVI_7925/1"], "led");
        assert_eq!(ids(&index, "clean", &marks).len(), 2, "no flags");
        assert_eq!(
            ids(&index, "move:shaky", &marks),
            vec!["MVI_7930/1"],
            "camera"
        );
        assert_eq!(ids(&index, "clip:mvi_79", &marks).len(), 4, "clip prefix");
        assert_eq!(
            ids(&index, "clip:MVI_7901/2", &marks),
            vec!["MVI_7901/2"],
            "segment id"
        );
        assert_eq!(
            ids(&index, "uncaptioned", &marks),
            vec!["MVI_7930/1"],
            "no caption"
        );
        Ok(())
    }

    #[test]
    fn metrics_are_relative_to_the_night() {
        let index = night();
        let marks = Marks::default();
        let strong = ids(&index, "fire>0.8", &marks);
        assert_eq!(
            strong.len(),
            2,
            "both fire shots are near the night's top: {strong:?}"
        );
        assert!(
            ids(&index, "fire>4.8%", &marks) == vec!["MVI_7901/1"],
            "raw share"
        );
        assert_eq!(ids(&index, "q>0.65", &marks).len(), 2, "quality");
        assert_eq!(
            ids(&index, "focus<0.8", &marks).len(),
            0,
            "all at the median"
        );
    }

    #[test]
    fn free_text_ranks_with_a_reason() {
        let index = night();
        let people = People::default();
        let marks = Marks::default();
        let n = Night::new(&index, &people, &marks);
        let hits = search(&Query::parse("spinner crowd"), &n).hits;
        assert_eq!(hits.len(), 2, "both spinner shots: {hits:?}");
        let top = hits.first();
        assert!(
            top.is_some_and(|h| h.reason.contains("caption") && h.reason.contains("spinner")),
            "the reason quotes the caption: {top:?}"
        );
        let poi = search(&Query::parse("poi hoodie"), &n).hits;
        assert_eq!(
            poi.len(),
            3,
            "everyone in the fixture wears the hoodie: {poi:?}"
        );
        let partial = search(&Query::parse("spinner zebra"), &n).hits;
        assert!(
            partial.iter().all(|h| h.score < 0.6),
            "half a match scores low: {partial:?}"
        );
        let syn = search(&Query::parse("flames"), &n).hits;
        assert_eq!(
            syn.len(),
            2,
            "`flames` is the fire facet (fire visible), not text: {syn:?}"
        );
        let audience = search(&Query::parse("audience"), &n).hits;
        assert_eq!(audience.len(), 2, "synonym of crowd: {audience:?}");
    }

    #[test]
    fn review_marks_filter() {
        let index = night();
        let mut marks = Marks::default();
        marks.set_verdict("MVI_7901/2", Some(Verdict::Pick));
        marks.set_verdict("MVI_7930/1", Some(Verdict::Reject));
        marks.set_rating("MVI_7925/1", 4);
        assert_eq!(ids(&index, "pick", &marks), vec!["MVI_7901/2"], "picked");
        assert_eq!(
            ids(&index, "reject", &marks),
            vec!["MVI_7930/1"],
            "rejected"
        );
        assert_eq!(ids(&index, "unreviewed", &marks).len(), 2, "unreviewed");
        assert_eq!(
            ids(&index, "rating>=3", &marks),
            vec!["MVI_7925/1"],
            "rated"
        );
    }

    #[test]
    fn more_like_this_prefers_the_twin() {
        let index = night();
        let marks = Marks::default();
        let like = ids(&index, "like:MVI_7901/1", &marks);
        assert_eq!(
            like.first().map(String::as_str),
            Some("MVI_7901/2"),
            "the near-identical shot first: {like:?}"
        );
        assert!(!like.contains(&"MVI_7901/1".to_owned()), "not itself");
        assert_eq!(
            like.last().map(String::as_str),
            Some("MVI_7930/1"),
            "the dark shaky one last: {like:?}"
        );
    }

    #[test]
    fn people_map_names_match() -> anyhow::Result<()> {
        let index = night();
        let marks = Marks::default();
        let people: People = toml::from_str("[names]\n\"MVI_7925\" = \"green-poi\"\n")?;
        let n = Night::new(&index, &people, &marks);
        let hits = search(&Query::parse("person:green-poi"), &n).hits;
        assert_eq!(hits.len(), 1, "only the mapped clip: {hits:?}");
        assert!(
            hits.first()
                .is_some_and(|h| h.reason.contains("person green-poi")),
            "named by the map: {hits:?}"
        );
        let hoodie = search(&Query::parse("person:hoodie"), &n).hits;
        assert_eq!(
            hoodie.len(),
            3,
            "caption descriptions match too: {hoodie:?}"
        );
        Ok(())
    }

    #[test]
    fn results_say_how_much_a_caption_query_could_see() -> anyhow::Result<()> {
        let index = night();
        let people = People::default();
        let marks = Marks::default();
        let n = Night::new(&index, &people, &marks);
        let text = search(&Query::parse("spinner"), &n);
        assert!(!text.complete, "free text cannot see the uncaptioned shot");
        assert_eq!(
            text.uncaptioned_in_scope, 1,
            "one segment has no caption: {text:?}"
        );
        let dark = search(&Query::parse("spinner -dark"), &n);
        assert!(
            dark.complete && dark.uncaptioned_in_scope == 0,
            "the uncaptioned shot is dark, so outside the scope: {dark:?}"
        );
        let facet = search(&Query::parse("shaky"), &n);
        assert!(
            facet.complete,
            "a measured facet sees every segment: {facet:?}"
        );
        assert_eq!(
            facet.uncaptioned_in_scope, 1,
            "the uncaptioned shot is still counted in scope: {facet:?}"
        );
        let moment = search(&Query::parse("moment:performing"), &n);
        assert!(!moment.complete, "caption fields are partial too");
        let negated = search(&Query::parse("-moment:performing"), &n);
        assert!(negated.complete, "a negated caption facet keeps them");
        Ok(())
    }
}
