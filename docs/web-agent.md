# An AI agent editing the document in the browser version

The page that embeds the browser version in an iframe (see [web-embedding.md](web-embedding.md)) can let its AI agent read and
edit the open presentation. The agent's edit goes through the editor's undo as the person's do, as one step, so 元に戻す takes it
back whole, and what the person changed stays. The same operations run on a file with no window: `ibis-agent` (crate
`crates/ibis-agent`).

Only presentations (`.pptx`) for now.

## Messages

The editor answers only its parent window, at the origin given explicitly with `?origin=https://app.example.com` (or
`window.ibisOfficeOptions.targetOrigin`); not the referrer's, and never `'*'`.

| The parent sends | The editor answers |
| --- | --- |
| `{ source: 'ibisoffice-agent', id, method: 'inspect' }` | `{ source: 'ibisoffice-agent', id, ok: true, result }`: the document as JSON text |
| `{ source: 'ibisoffice-agent', id, method: 'apply', params: [operations] }` | `result`: `{ created, document }` as JSON text |
| `{ source: 'ibisoffice-agent', id, method: 'state' }` | `result`: `{ dirty, revision }` as JSON text; `revision` changes with every edit, undo and redo (cheap: for saving once the person stops) |
| `{ source: 'ibisoffice-agent', id, method: 'save' }` | `result`: the file's bytes (an `ArrayBuffer`, transferred); the document is then saved (not dirty). What the person is typing is saved as typed and their typing goes on |

A refused request is answered `{ ok: false, error }`, and nothing changed. Once the document is open the editor posts
`{ source: 'ibisoffice-agent', type: 'ready' }`.

`?open=message` takes the document from the parent instead of a URL: the editor posts `{ source: 'ibisoffice-agent', type:
'need-file' }`, and the parent answers `{ source: 'ibisoffice-agent', type: 'file', name, bytes }` (`bytes` an `ArrayBuffer`).

## The document

`inspect` gives the slide size and the layouts' names, and the slides in order, each with its `id`, `number`, `layout` and its
shapes: `id`, `kind` (`text_box`, `shape`, `picture`, `line`, `table`, `chart`, `other`), `placeholder` (`ctrTitle`, `title`,
`body`, `subTitle`…), place and size in points (`x`, `y`, `w`, `h`), `text` (paragraphs joined by `\n`), `font_size`, a table's
`cells`. `dirty` says whether there are changes not saved.

The ids stay while the document is open. A file read again is numbered again (the same bytes give the same ids).

## Operations

A list is applied whole or not at all: an operation that fails, or a result the file cannot be saved with yet, is refused with
the operation's `index` and why. Places are in points, colours `#RRGGBB`. A slide or a shape is named by its id, or by the `as`
name of one an earlier operation of the list made; a shape also as `{ "placeholder": "title" }` (the one of its slide).

| `op` | Fields |
| --- | --- |
| `add_slide` | `index?` (0-based; the end), `layout?` (by name; the slide before's), `as?` |
| `delete_slide` | `slide` |
| `move_slide` | `slide`, `index` |
| `duplicate_slide` | `slide`, `index?`, `as?` |
| `set_notes` | `slide`, `text` |
| `set_background` | `slide`, `color` |
| `add_text_box` | `slide`, `frame` `{x, y, w, h}`, `text`, `format?`, `as?` |
| `add_shape` | `slide`, `geometry` (`rect`, `roundRect`, `ellipse`, `triangle`, `rightArrow`…), `frame`, `fill?`, `line?`, `text?`, `as?` |
| `add_table` | `slide`, `rows` (the cells' text by row), `frame?`, `as?` |
| `delete_shape` | `slide`, `shape` |
| `set_frame` | `slide`, `shape`, `x?`, `y?`, `w?`, `h?` |
| `set_text` | `slide`, `shape`, `text` (each paragraph keeps the format of the one it replaces) |
| `replace_text` | `find`, `replace`, `slide?`, `shape?` (shapes and table cells; says how many it `replaced`) |
| `format_text` | `slide`, `shape`, `format` `{bold?, italic?, underline?, size?, color?, font?}`, `match?`, `cell?` `[row, column]` |
| `format_paragraphs` | `slide`, `shape`, `format` `{align?, list? (none, bullet, numbered), level?, space_before?, space_after?}`, `paragraphs?`, `cell?` |
| `set_fill` | `slide`, `shape`, `color` (`null`: none) |
| `set_line` | `slide`, `shape`, `color` (`null`: none), `width?` |
| `set_cell` | `slide`, `shape`, `row`, `column`, `text` |
| `insert_table_row` | `slide`, `shape`, `at?` |
| `delete_table_row` | `slide`, `shape`, `at` |

## With no window

```sh
ibis-agent new deck.pptx                      # an empty presentation
ibis-agent inspect deck.pptx                  # the document, as `inspect` gives it
ibis-agent apply deck.pptx ops.json -o out.pptx   # `-` reads the operations from standard input
```

`apply` prints `{ "replaced": n, "document": … }` of the file written, or `{ "error": … }` and exits with 1, writing nothing.

`scripts/build-agent-wasm.sh` builds the same for a JavaScript runtime (wasm-bindgen's `nodejs` target, in
`target/agent-wasm/`): `new_presentation()`, `inspect(bytes, name)` and `apply(bytes, name, ops)` (the file written and how
many places were `replaced`; a refusal throws the error as JSON).
