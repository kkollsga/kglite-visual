# Getting started

The workspace in this guide is currently available from the
[PR 4 source preview](https://github.com/kkollsga/kglite-visual/pull/4).
PyPI `0.1.7` is the released viewer and does not yet contain the
Explore, Data and Query experience shown here.

(preview-the-workspace)=
## Preview the workspace

The preview needs Rust and Node.js. Check out the reviewed revision, build the
frontend that the Rust binary embeds, then open the included sample:

```bash
git clone https://github.com/kkollsga/kglite-visual.git
cd kglite-visual
git fetch origin pull/4/head
git checkout 8870e9643bb12c930475619fe0ffe6f3291ad40f
npm --prefix frontend ci
npm --prefix frontend run build
cargo run -p kglite-visual-cli -- docs/_static/team.kgl
```

This revision includes the preview and the sample.

The command opens a localhost browser tab. If it cannot, copy the `url` from
the single JSON line it prints. The {ref}`CLI reference <serve>`
documents ports, `--no-open`, load limits and clean shutdown.

## Try one action

The opening Explore view is a schema: three type nodes (`Person`, `Project` and
`Skill`) connected by four relationship types. Choose **Person**, then choose
**Browse Person instances**. The workspace loads nine named people and no
relationships; browsing a type loads its records without inducing edges.

That is the central interaction: inspect the size and shape first, then ask the
server to load a bounded slice. Continue with the
**[first exploration](first-exploration.md)** to inspect Ada, filter the team,
calculate degree, save the result and export it.

You can also [download `team.kgl`](_static/team.kgl) separately and open it with
any preview binary built from the pinned revision.

## Published package

For the released `0.1.7` behavior:

```bash
pip install kglite-visual==0.1.7
kglite-visual graph.kgl
```

Use the [stable documentation](https://kglite-visual.readthedocs.io/en/stable/)
with that package. A `.kgl` file must use a format supported by the embedded
kglite reader; it need not come from the exact same release. The current source
tree pins kglite 0.17.1 and reads compatible files written by older releases.
Packaging details, in-memory graph handoff and notebooks live in the
[Python API](python.md).
