# Getting started

Install the released viewer, download the small sample, and open it:

```bash
python -m pip install kglite-visual==0.1.9
curl -L https://kglite-visual.readthedocs.io/en/stable/_static/team.kgl -o team.kgl
kglite-visual team.kgl
```

You can also [download `team.kgl`](_static/team.kgl) in your browser.

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

For your own data, replace `team.kgl` with any compatible graph path. A `.kgl`
file must use a format supported by the embedded
kglite reader; it need not come from the exact same release. The current source
tree pins kglite 0.17.4 and reads compatible files written by older releases.
Packaging details, in-memory graph handoff and notebooks live in the
[Python API](python.md).
