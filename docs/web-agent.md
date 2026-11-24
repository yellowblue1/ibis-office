# An AI agent editing the document in the browser version

The page that embeds the browser version in an iframe (see [web-embedding.md](web-embedding.md)) can let its AI agent read and
edit the open file. The agent's edit goes through the editor's undo as the person's do, as one step, so 元に戻す takes it back
whole, and what the person changed stays. The same operations run on a file with no window: `ibis-agent` (crate
`crates/ibis-agent`).

A presentation (`.pptx`) and a document (`.docx`) are edited, each with a summary and operations of its own (below); the
summary's `kind` (`presentation`, `document`) says which the file is.

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

## A presentation

`inspect` gives the slide size and the layouts' names, and the slides in order, each with its `id`, `number`, `layout` and its
shapes: `id`, `kind` (`text_box`, `shape`, `picture`, `line`, `table`, `chart`, `other`), `placeholder` (`ctrTitle`, `title`,
`body`, `subTitle`…), place and size in points (`x`, `y`, `w`, `h`), `text`, `font_size`, a table's `cells` (each cell's
text, by row) and the slide's `notes`. `dirty` says whether there are changes not saved.

Text is its paragraphs: a list of strings, read and given the same way (`["Title", "", "Date"]` is three paragraphs, the
second empty). A paragraph holds no line break; one is refused.

The ids stay while the document is open. A file read again is numbered again (the same bytes give the same ids).

### Operations

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
| `add_table` | `slide`, `rows` (each cell's text, by row), `frame?`, `as?` |
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

## A document

`inspect` gives `styles`, the paragraph styles by name (`Normal`, `Title`, `Subtitle`, `Heading 1` to `Heading 3`, then the
document's others), and the body's `blocks` in order: a paragraph `{ id, type: "paragraph", text, style?, list?, level? }`
(`style` when it is not `Normal`; `list` `bullet` or `numbered` and its `level`, 0 to 8, for a list item), a table `{ id,
type: "table", cells }` (each cell's paragraphs, by row). `header` and `footer` are the last section's header's and
footer's paragraphs, read only. `dirty` says whether there are changes not saved.

A paragraph's `text` is one string. A line break within it (Shift+Enter) reads as `\n`, and is not written: text given holds
no line break (one is refused); where there are several paragraphs they are given as a list of strings, as a presentation's
text is. A tab is `\t`; a picture or a field is one character (U+FFFC).

The ids stay while the document is open. A file read again is numbered again (the same bytes give the same ids).

### Operations

Operations are applied as a presentation's are: a list whole or not at all, a refusal with the operation's `index` and why.
Sizes are in points, colours `#RRGGBB`. A paragraph or a table of the body is named by its id, or by the `as` name of one an
earlier operation of the list made. A document that records changes (変更履歴の記録), is protected against editing or is
marked as final is not edited.

| `op` | Fields |
| --- | --- |
| `insert_paragraphs` | `text` (paragraphs), `after?` or `before?` (a block; neither: at the end), `style?` (by name; `Normal`), `as?` (names the first); `created` lists all their ids |
| `set_text` | `paragraph`, `text` (one paragraph's: it takes the format of its first character; a paragraph with a picture, a field or a comment's mark is changed with `replace_text`) |
| `delete` | `blocks` (paragraphs and tables; the document keeps a paragraph at its end) |
| `replace_text` | `find`, `replace`, `paragraph?` (a paragraph or a table; else the body and its tables' cells; says how many it `replaced`) |
| `format_text` | `paragraph`, or `table` and `cell` `[row, column]`; `format` `{bold?, italic?, underline?, size?, color?, font?}`, `match?` |
| `format_paragraphs` | `paragraphs` (a table: its cells'), `format` `{align? (left, center, right, justify), list? (none, bullet, numbered), level?, space_before?, space_after?}` |
| `set_style` | `paragraphs` (a table: its cells'), `style` (a name of `styles`; Word's usual ones the file lacks are defined as Word's default style set has them) |
| `add_table` | `rows` (each cell's paragraphs, by row), `after?` or `before?`, `as?` (across the text's width, as 挿入 > 表 makes it; an empty paragraph follows a table at the end) |
| `set_cell` | `table`, `row`, `column` (the row's cells, 0-based), `text` |
| `insert_table_row` | `table`, `at?` (0-based; the end) |
| `delete_table_row` | `table`, `at` |

## With no window

```sh
ibis-agent new deck.pptx                      # an empty presentation (`.docx`: a document of one empty paragraph)
ibis-agent inspect deck.pptx                  # the file, as `inspect` gives it
ibis-agent apply deck.pptx ops.json -o out.pptx   # `-` reads the operations from standard input
```

`apply` prints `{ "replaced": n, "document": … }` of the file written, or `{ "error": … }` and exits with 1, writing nothing.

`scripts/build-agent-wasm.sh` builds the same for a JavaScript runtime (wasm-bindgen's `nodejs` target, in
`target/agent-wasm/`): `new_presentation()`, `inspect(bytes, name)` and `apply(bytes, name, ops)` (the file written and how
many places were `replaced`; a refusal throws the error as JSON).
