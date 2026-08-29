//! `kglite-visual render` — Cypher in, image out, with no server in the loop
//! (plan D13).
//!
//! **A thin face over `core::render`.** Everything this file does is turn
//! flags into a [`RenderRequest`], write the bytes, and print one line. It
//! contains no drawing, no layout and no bound of its own; if a rule about what
//! an image may contain ever appears here, it is in the wrong crate.
//!
//! **stdout discipline, same as the server's** (`cli.rs`): stdout carries
//! exactly one line — the render's JSON summary — and nothing else, ever.
//! Diagnostics go to stderr, and a failed render prints *nothing* on stdout, so
//! a harness that reads a line got a render.

use std::path::{Path, PathBuf};
use std::time::Duration;

use kglite_visual_core::render::{ExpandSource, RenderFormat, RenderRequest, RenderSource, Theme};
use kglite_visual_core::request::{CypherRequest, EdgeDirection};
use kglite_visual_core::{load_graph, render, GraphSource, QueryConfig};
use serde::Serialize;

/// `--format`.
///
/// A local mirror of [`RenderFormat`] rather than a `clap::ValueEnum` on the
/// core type: `kglite-visual-core` is transport- *and* face-agnostic, and a
/// `clap` derive on one of its enums would make the argument parser part of the
/// engine's public API. Two variants and one `From`; the compiler catches a
/// third being added on either side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum FormatArg {
    Svg,
    Png,
}

impl From<FormatArg> for RenderFormat {
    fn from(arg: FormatArg) -> Self {
        match arg {
            FormatArg::Svg => RenderFormat::Svg,
            FormatArg::Png => RenderFormat::Png,
        }
    }
}

/// `--theme`. Same argument as [`FormatArg`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ThemeArg {
    Dark,
    Light,
}

impl From<ThemeArg> for Theme {
    fn from(arg: ThemeArg) -> Self {
        match arg {
            ThemeArg::Dark => Theme::Dark,
            ThemeArg::Light => Theme::Light,
        }
    }
}

#[derive(clap::Args, Debug)]
pub struct RenderArgs {
    /// The `.kgl` file to draw.
    pub file: PathBuf,

    /// Draw the type-level meta-graph — the app's entry screen. The default
    /// when no other source is named.
    #[arg(long, group = "source")]
    pub meta: bool,

    /// Draw the graph a read-only Cypher query returns.
    ///
    /// The query must RETURN nodes, relationships or paths: a table has no
    /// picture, and this says so rather than emitting an empty canvas.
    #[arg(long, group = "source", value_name = "QUERY")]
    pub cypher: Option<String>,

    /// Draw a bounded neighbourhood expansion, as `type=T [rel=R] [dir=out|in|both]`.
    ///
    /// `type` is a node type from the meta-graph. `rel` narrows to one
    /// relationship type; omitted, the walk follows every one, which is the
    /// expensive case. `dir` defaults to `both`.
    #[arg(long, group = "source", value_name = "KEY=VALUE", num_args = 1..)]
    pub expand: Option<Vec<String>>,

    #[arg(long, value_enum, default_value = "svg")]
    pub format: FormatArg,

    /// Where to write the image. Defaults to a name derived from the graph and
    /// the source, in the current directory.
    #[arg(long, short = 'o', value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Layout seed. Reaches the initial placement only; the force pass has no
    /// randomness at all, so the same seed is the same image forever.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,

    #[arg(long, default_value_t = kglite_visual_core::render::DEFAULT_WIDTH)]
    pub width: u32,

    #[arg(long, default_value_t = kglite_visual_core::render::DEFAULT_HEIGHT)]
    pub height: u32,

    /// Dark matches the app; light is for a white page.
    #[arg(long, value_enum, default_value = "dark")]
    pub theme: ThemeArg,

    /// Rows (for `--cypher`) or nodes (for `--expand`) wanted. Clamped in core
    /// to the response bound, whatever is asked for.
    #[arg(long)]
    pub limit: Option<u32>,

    /// Wall-clock ceiling for the query behind `--cypher`, in seconds.
    #[arg(long, default_value_t = 30)]
    pub query_timeout_secs: u64,
}

/// The single stdout line.
///
/// Field names are the contract an agent parses by name, exactly as
/// `LaunchInfo`'s are. `out` first because it is the one a shell pipeline
/// wants.
#[derive(Debug, Serialize)]
struct RenderSummary<'a> {
    out: String,
    format: &'a str,
    width: u32,
    height: u32,
    nodes: u32,
    links: u32,
    /// True when a bound clipped this answer. The banners say what was clipped;
    /// they are also drawn into the image, because an image travels without its
    /// response (D5).
    truncated: bool,
    banners: &'a [String],
    bytes: usize,
}

pub fn run(args: &RenderArgs) -> Result<(), Box<dyn std::error::Error>> {
    let source = parse_source(args)?;
    // Load before anything else: a summary line for a graph that turned out to
    // be unreadable would name a file nothing wrote.
    let graph = load_graph(GraphSource::Path(&args.file))?;
    let request = RenderRequest {
        source,
        format: args.format.into(),
        width: args.width,
        height: args.height,
        seed: args.seed,
        theme: args.theme.into(),
    };
    let out = args
        .out
        .clone()
        .unwrap_or_else(|| default_out_path(args, request.format));

    let rendered = render(
        &graph,
        &args.file.display().to_string(),
        QueryConfig {
            timeout: Duration::from_secs(args.query_timeout_secs),
        },
        &request,
    )?;

    for banner in &rendered.banners {
        // The banner is in the image too. It is here as well because a shell
        // user watching a render scroll past should not have to open the file
        // to learn the answer was clipped.
        eprintln!("kglite-visual: {banner}");
    }
    std::fs::write(&out, &rendered.bytes)?;

    let summary = RenderSummary {
        out: out.display().to_string(),
        format: rendered.format.extension(),
        width: rendered.width,
        height: rendered.height,
        nodes: rendered.nodes,
        links: rendered.links,
        truncated: rendered.truncated,
        banners: &rendered.banners,
        bytes: rendered.bytes.len(),
    };
    // THE one stdout line, printed after the file is on disk: a harness that
    // reads it can open the path immediately.
    println!("{}", serde_json::to_string(&summary)?);
    use std::io::Write as _;
    std::io::stdout().flush()?;
    Ok(())
}

fn parse_source(args: &RenderArgs) -> Result<RenderSource, Box<dyn std::error::Error>> {
    if let Some(query) = &args.cypher {
        return Ok(RenderSource::Cypher(CypherRequest {
            query: query.clone(),
            params: Default::default(),
            limit: args.limit,
            // Forced, not defaulted: a render of a table is not a thing.
            as_graph: true,
        }));
    }
    if let Some(pairs) = &args.expand {
        return Ok(RenderSource::Expand(parse_expand(pairs, args.limit)?));
    }
    // `--meta` is also the default. Naming it explicitly is still worth a flag:
    // a script that says what it is asking for keeps saying it when the default
    // moves.
    Ok(RenderSource::Meta)
}

/// `type=Wellbore rel=HAS_CORE dir=out` into an [`ExpandSource`].
///
/// Key/value pairs rather than three flags because the three belong together —
/// `rel` and `dir` mean nothing without `type` — and a flag triple lets a caller
/// write two of them and get a silently different query.
fn parse_expand(
    pairs: &[String],
    limit: Option<u32>,
) -> Result<ExpandSource, Box<dyn std::error::Error>> {
    let mut node_type: Option<String> = None;
    let mut relationship: Option<String> = None;
    let mut direction = EdgeDirection::default();

    for pair in pairs {
        let Some((key, value)) = pair.split_once('=') else {
            return Err(format!(
                "--expand takes key=value pairs; {pair:?} has no '='. \
                 Try: --expand type=Wellbore rel=HAS_CORE dir=out"
            )
            .into());
        };
        match key {
            "type" => node_type = Some(value.to_string()),
            "rel" => relationship = Some(value.to_string()),
            "dir" => {
                direction = match value {
                    "out" => EdgeDirection::Out,
                    "in" => EdgeDirection::In,
                    "both" => EdgeDirection::Both,
                    other => {
                        return Err(
                            format!("--expand dir must be out, in or both; got {other:?}").into(),
                        )
                    }
                }
            }
            other => {
                return Err(
                    format!("--expand knows the keys type, rel and dir; got {other:?}").into(),
                )
            }
        }
    }

    let node_type = node_type.ok_or("--expand needs type=<NodeType>")?;
    Ok(ExpandSource {
        node_type,
        relationship,
        direction,
        limit,
    })
}

/// `graph.kgl` + `--meta` -> `./graph-meta.svg`.
///
/// Derived from the *source* as well as the file, so rendering three views of
/// one graph does not silently overwrite the first two.
fn default_out_path(args: &RenderArgs, format: RenderFormat) -> PathBuf {
    let stem = args
        .file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "graph".to_string());
    let suffix = if args.cypher.is_some() {
        "cypher".to_string()
    } else if let Some(pairs) = &args.expand {
        let named = pairs
            .iter()
            .find_map(|p| p.strip_prefix("type="))
            .unwrap_or("expand");
        format!("expand-{}", sanitize(named))
    } else {
        "meta".to_string()
    };
    Path::new(&format!("{stem}-{suffix}.{}", format.extension())).to_path_buf()
}

/// A type name is data, and data reaches this function on its way to a
/// filesystem path. Anything that is not a plain filename character becomes
/// `_`, so a type called `../../etc/passwd` cannot name the output file.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(expand: Option<Vec<&str>>, cypher: Option<&str>) -> RenderArgs {
        RenderArgs {
            file: PathBuf::from("demo.kgl"),
            meta: false,
            cypher: cypher.map(str::to_string),
            expand: expand.map(|v| v.into_iter().map(str::to_string).collect()),
            format: FormatArg::Svg,
            out: None,
            seed: 0,
            width: 800,
            height: 600,
            theme: ThemeArg::Dark,
            limit: None,
            query_timeout_secs: 30,
        }
    }

    #[test]
    fn expand_pairs_parse_into_the_documented_request() {
        let source = parse_source(&args(
            Some(vec!["type=Wellbore", "rel=HAS_CORE", "dir=out"]),
            None,
        ))
        .expect("the documented form");
        let RenderSource::Expand(expand) = source else {
            panic!("wrong source variant");
        };
        assert_eq!(expand.node_type, "Wellbore");
        assert_eq!(expand.relationship.as_deref(), Some("HAS_CORE"));
        assert_eq!(expand.direction, EdgeDirection::Out);
    }

    #[test]
    fn a_malformed_expand_pair_is_refused_with_a_usable_message() {
        for bad in [
            vec!["Wellbore"],
            vec!["kind=Wellbore"],
            vec!["type=W", "dir=sideways"],
        ] {
            let err = parse_source(&args(Some(bad.clone()), None))
                .expect_err("must refuse")
                .to_string();
            assert!(!err.is_empty(), "{bad:?} produced an empty message");
        }
        let err = parse_source(&args(Some(vec!["rel=HAS_CORE"]), None))
            .expect_err("rel without type is not a request")
            .to_string();
        assert!(err.contains("type="), "{err}");
    }

    #[test]
    fn a_cypher_render_is_always_a_graph_render() {
        // `as_graph: false` returns a table, and a table has no picture. A
        // caller cannot ask for the unrenderable variant by accident.
        let RenderSource::Cypher(cypher) =
            parse_source(&args(None, Some("MATCH (n) RETURN n"))).expect("cypher")
        else {
            panic!("wrong source variant");
        };
        assert!(cypher.as_graph);
    }

    #[test]
    fn the_default_output_name_separates_the_three_sources() {
        assert_eq!(
            default_out_path(&args(None, None), RenderFormat::Svg),
            PathBuf::from("demo-meta.svg")
        );
        assert_eq!(
            default_out_path(&args(None, Some("MATCH (n) RETURN n")), RenderFormat::Png),
            PathBuf::from("demo-cypher.png")
        );
        assert_eq!(
            default_out_path(&args(Some(vec!["type=Wellbore"]), None), RenderFormat::Svg),
            PathBuf::from("demo-expand-Wellbore.svg")
        );
    }

    #[test]
    fn a_type_name_cannot_steer_the_output_path() {
        // The name comes out of a `.kgl` someone else built.
        let path = default_out_path(
            &args(Some(vec!["type=../../etc/passwd"]), None),
            RenderFormat::Svg,
        );
        assert_eq!(path, PathBuf::from("demo-expand-______etc_passwd.svg"));
        assert!(!path.to_string_lossy().contains(".."));
    }
}
