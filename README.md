> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.

# Santh GPUI

A reusable GPU-accelerated UI framework fork derived from
[Zed](https://github.com/zed-industries/zed).

The canonical repository is the private
[`santhreal/gpui`](https://github.com/santhreal/gpui) repository. Framework changes
are maintained here. Applications keep their own surfaces, themes, and domain
logic in their repositories.

## Packages

- `gpui`: entities, windows, elements, layout, input, and application contexts.
- `gpui_platform`: native platform integration.
- `gpui_wgpu`: the wgpu renderer, text system, and offscreen rendering.

The repository retains the upstream workspace and commit history. Applications
depend on the GPUI packages rather than the Zed editor application.

## Use from another Rust project

Repository access and Git authentication are required. Pin a revision:

```toml
[dependencies]
gpui = { git = "ssh://git@github.com/santhreal/gpui.git", rev = "d9a8bdbcc23b4c619e186a193d147bcad82b8e69" }
```

Use the same revision for companion packages such as `gpui_platform` and
`gpui_wgpu`. All consuming projects resolve framework packages from this
repository. Do not copy framework sources into application repositories.

## Source reference

- [GPUI package](crates/gpui)
- [Platform integration](crates/gpui_platform)
- [wgpu renderer](crates/gpui_wgpu)

## Licensing

GPUI is licensed under Apache-2.0. Other components retain the licenses specified
in their crate manifests; the upstream workspace also includes GPL-3.0-or-later
components. Preserve the upstream license and copyright notices.
