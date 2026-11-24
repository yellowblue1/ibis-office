# An AI agent editing the document in the browser version

The page that embeds the browser version in an iframe (see [web-embedding.md](web-embedding.md)) can let its AI agent read and
edit the open file. The agent's edit goes through the editor's undo as the person's do, as one step, so 元に戻す takes it back
whole, and what the person changed stays. The same operations run on a file with no window: `ibis-agent` (crate
`crates/ibis-agent`).

Each kind of file has its summary and operations of its own: a presentation's (`.pptx`), a document's (`.docx`) and a
workbook's (`.xlsx`), below. The summary's `kind` says which (`presentation`, `document`, `workbook`); operations of another kind
are refused.

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

## A workbook

`inspect` gives the sheets in the workbook's order, each with its `id`, `name`, `number` (1-based, hidden sheets counted),
`hidden` (a hidden sheet has no `id` and is not edited), `other` (a chart sheet: no cells), `protected` (シートの保護: its
cells are not changed), `used` (the range from the first to the last cell with something in it, `A1:F42`), `merged` (the
merged ranges) and its `cells` with a value or a formula, by row: `at` (`B2`), `value` (a string, a number or a logical
value; an error as its text, `#DIV/0!`), `formula` (`=SUM(B3:B9)`) and `text`, what the cell shows when that is not the value
written out (a number format, a date: `{"at": "A5", "value": 46350, "text": "2026/11/24"}`). A sheet lists at most 2000 cells,
the first rows first; `more` says how many were left out.

The ids stay while the workbook is open. A file read again is numbered again (the same bytes give the same ids).

A sheet is named by its id, its name (in any case, as Excel), or the `as` name of one an earlier operation of the list made.
Cells and ranges are in A1 notation (`B2`, `A1:C3`), rows by their numbers (`1:3`), columns by their letters (`A:C`), colours
`#RRGGBB`. A list is applied whole or not at all, as a presentation's; the editor then shows the sheet the last operation named.

`set_cells` puts values as a person typing them would: a string starting with `=` is a formula (`=sum(a1:a3)` is entered as
`=SUM(A1:A3)`; one that does not read is refused); a number, a percentage or a date written as typed (`1,234`, `50%`,
`2026/11/24`) is that number, and a cell of General takes the number format Excel gives it; `TRUE` and `FALSE` are logical
values; a leading `'` keeps the rest as text, a sign-led word (`-x`) is text, and in a cell formatted as text (`@`) everything
is. A JSON number or logical value is put as it is; `null` or `""` empties the cell. What depends on the cells is calculated
again. The editor's AutoCorrect and the links it makes of typed addresses are not applied.

| `op` | Fields |
| --- | --- |
| `set_cells` | `sheet`, `at` (the top-left cell), `values` (rows of values) |
| `clear` | `sheet`, `range` (values and formulas; the formats stay) |
| `add_sheet` | `name?` (`SheetN`, the first not taken), `index?` (0-based; the end), `as?` |
| `rename_sheet` | `sheet`, `name` (1 to 31 characters, none of `[ ] : * ? / \`, not another sheet's) |
| `delete_sheet` | `sheet` (not the last one shown; formulas that refer to it get `#REF!`) |
| `move_sheet` | `sheet`, `index` (0-based, its place after the move) |
| `insert_rows` | `sheet`, `at` (1-based: the rows go before it), `count?` (1) |
| `delete_rows` | `sheet`, `at`, `count?` |
| `insert_columns` | `sheet`, `at` (`C`), `count?` |
| `delete_columns` | `sheet`, `at`, `count?` |
| `format_cells` | `sheet`, `range`, `format` `{bold?, italic?, underline?, size?, color?, font?, fill? (null: none), number_format? ("#,##0", "0.0%", "yyyy/mm/dd"…), align? (left, center, right), wrap?}` |
| `merge` | `sheet`, `range` (the cells but the top-left one are emptied) |
| `unmerge` | `sheet`, `range` (the merged ranges within it) |
| `set_column_width` | `sheet`, `columns` (`A:C`), `width` (characters, 0 to 255; 0 hides) |
| `set_row_height` | `sheet`, `rows` (`1:3`), `height` (points, 0 to 409; 0 hides) |
| `replace_text` | `find`, `replace`, `sheet?` (the text cells of every shown sheet; not formulas; says how many it `replaced`) |

`add_sheet` answers the new sheet's id in `created` (`{"sheet": 3}`).

A cell the person is typing in is entered before an `apply`; `inspect` and `save` give the workbook without it until it is
entered.

## With no window

```sh
ibis-agent new deck.pptx                      # an empty file of the kind the name says: a presentation of one title slide,
                                              # a document of one empty paragraph (.docx), a workbook of one empty sheet (.xlsx)
ibis-agent inspect deck.pptx                  # the file, as `inspect` gives it
ibis-agent apply deck.pptx ops.json -o out.pptx   # `-` reads the operations from standard input
```

`apply` prints `{ "replaced": n, "document": … }` of the file written, or `{ "error": … }` and exits with 1, writing nothing.
