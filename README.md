# Luau

Highlighting for [Luau](https://luau.org) files (`.luau` and `.lua`), and
[luau-lsp](https://github.com/JohnnyMorganz/luau-lsp) for diagnostics, hover, go to definition,
references, symbols, and completion, with Roblox's types and API docs.

## Setting up

Install luau-lsp so that `luau-lsp` is on the `PATH`: download it from its
[releases](https://github.com/JohnnyMorganz/luau-lsp/releases), or use Rokit
(`rokit add --global JohnnyMorganz/luau-lsp`). A project's own Rokit or Aftman tools work too,
in the folders that list them. To use one elsewhere, set its path:

```toml
[lsp.servers.luau-lsp]
command = "C:/tools/luau-lsp.exe"
```

In Rojo projects, install the **Rojo** extension too: it keeps the `sourcemap.json` luau-lsp
resolves requires with.

## Roblox

With the **Platform** setting on `roblox` (the default), luau-lsp knows Roblox's globals, classes,
and enums, and shows their documentation. Flareite downloads the type definitions and API docs
from [luau-lsp.pages.dev](https://luau-lsp.pages.dev) the first time, keeps them, and gets them
again once they're a day old. **Roblox Security Level** picks which APIs it knows:
`PluginSecurity` (the default) covers games and plugins.

**Sourcemaps.** luau-lsp resolves `require(script.Parent.Module)` and instance paths with a
`sourcemap.json` that Rojo writes. The **Rojo** extension keeps it up to date in Rojo projects;
luau-lsp picks up its changes as they're written.

## `end` typed for you

As in Roblox Studio, Enter after a line that opens a block (`function f()`, `local function`,
`if x then`, `for ... do`, `while ... do`, `do`) puts the cursor on an indented line with the
block's `end` below it, when the block doesn't have one yet:

```luau
if ready then
    |
end
```

Nothing is added when the blocks already balance, or when the block already has a body below it.
Closing brackets after the cursor go after `end`, as in `pcall(function() ... end)`. Undo takes
back just the `end`. Reading the document as Enter is pressed is why the extension asks to read
documents.

## Plain Luau

Set **Platform** to `standard` for Luau outside Roblox: no Roblox types.

## More settings

luau-lsp's other settings (as VS Code names them) go in `settings.toml`, or in a folder's
`.flareite/settings.toml`:

```toml
[lsp.servers.luau-lsp.settings]
"luau-lsp.require.mode" = "relativeToFile"
"luau-lsp.completion.imports.enabled" = true
```

`.luaurc` files are JSON (highlighted when the JSON extension is installed), checked as you edit
them, and their keys complete.

**Luau: Restart Language Server** restarts luau-lsp, for example after installing it.

## Known gaps

The grammar (tree-sitter-luau 1.2.0) predates function attributes: `@native` is colored, but the
line it's on is parsed as an error.
