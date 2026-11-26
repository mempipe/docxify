# The demo

A browser editor for DOCX, built on the engine through its WebAssembly bindings. The editor itself is the package [`@mempipe/docxify-editor`](../../packages/editor/README.md), and this is a page that uses it. It shows a document on pages, as Word lays it out, and you edit on the pages. One person can edit alone, or several can edit one document together.

It is a demo. It has no sign-in and no storage, and it was tried in headless Chrome only.

| Path | What it is |
| --- | --- |
| `web/` | The demo page, the collaboration server, and two Node programs that check the editor |
| `scripts/` | Checks that drive the demo in headless Chrome, and comparisons with Word and python-docx |
| `fixtures`, `target` | Links to the repository's `fixtures/` and to Cargo's `target/` |

`scripts/` is the prototype's, unchanged. `web/` holds the prototype's files that are not the editor, edited to import the package. Both expect to be run from this directory, with `fixtures/` and `target/` beside them. That is why the two links exist. Some comments in them give paths in the prototype's own repository. Read those as relative to this directory.

## Running it

`proto use` at the repository's root installs Node.js and pnpm. Then build the demo:

```bash
moon run demo:build
```

This installs the npm packages and `wasm-bindgen-cli`, builds the editor package with its WebAssembly module, and bundles the page into `web/dist/`.

Everything below runs from this directory, `apps/demo`.

```bash
node web/collab-server.ts                   # http://127.0.0.1:8766/
node web/collab-server.ts --host 0.0.0.0    # reachable from other devices
```

- Alone: `http://127.0.0.1:8766/web/index.html`
- Shared: `http://127.0.0.1:8766/web/index.html?collab`

Type in the text, and press Download DOCX. Use `?fixture=real-world/python-docx.docx` to open another fixture.

## Editing

The page shows the document on pages, laid out as Word lays it out, and you edit on the pages. Click to place the caret, and drag or Shift+click to select. Up and Down move by the lines on the page. Double-click a header or footer to edit it, and double-click the body to go back.

Each story keeps a hidden ProseMirror view (the package's `editor` module) that takes the keyboard. ProseMirror never holds the document. Its state is a view built from the inspection, and each character carries a `leaf` mark naming its engine text node. Every edit becomes engine commands first, and the commands go to a replica of the document's editing state, whether one person edits or several:

- Typing sends one `ReplaceRange` for the text being replaced. At a caret, both ends name the same place, including in an empty paragraph.
- Enter splits the paragraph and Shift+Enter inserts a line break. Backspace at a paragraph's start joins it to the previous one, and Delete at its end joins the next one.
- Deleting a shaded item removes it with `RemoveNode`: a tab, a field, a tracked insertion or deletion, a drawing or a preserved element. For revisions already in the source file, removing an insertion rejects it and removing a deletion accepts it. Removing a revision created in this session is refused.
- After a command that adds or removes nodes, the story is drawn again from the engine.
- Undo and redo take back and restore your own changes, through the replica.
- The engine refuses what it cannot do safely, and the page says why. Examples are joining across a section break or a table, and typing inside a field result. The view stays as it was.
- A caret beside a field is outside it. Text typed between two fields goes between them, and each field keeps its own result. A range that covers a whole result removes the whole field. A range that cuts into an instruction or a result is refused.
- Deleting, typing, Enter or cut over a selection that spans paragraphs becomes `ReplaceRange`. So does any paste, and ordinary keyboard input over a selection in one paragraph: the whole selection is replaced with the whole text, even where the two share letters. Copying writes one line per paragraph, so pasting a copy makes the same paragraphs. A copy holds what the body shows: a filled control's value, tabs and line breaks, and not the words a tracked change deleted. Pasted text is plain, and a pasted tab becomes a space. A paste with no text and no image the document can hold changes nothing, and the page says what can be pasted.
- ⌘B, ⌘I and ⌘U, or the B, I and U buttons, become `FormatRange` over the selection. As in Word, a property is turned off when all the selected text has it, and on otherwise. The buttons show what the selection has. With nothing selected, the page says to select text first.
- The style picker, ⌘⌥1 to ⌘⌥3 for headings and ⌘⌥0 for the default style become `SetParagraphStyle` over the selected paragraphs, or the caret's paragraph. The picker shows the style the selected paragraphs share, and is blank when they differ.
- The bullets and numbering buttons, ⌘⇧8 and ⌘⇧7 become `SetList` over the selected paragraphs, or the caret's paragraph. As in Word, a list is taken off when every paragraph already has that kind, and the buttons show the kind they share. Tab and Shift+Tab at the start of a list item, or over a selection that spans paragraphs, become `ChangeListLevel`. Enter in an empty list item ends the list, and Backspace at the start of one takes it out of the list.
- The link button and ⌘K open an address field below the selection, and Enter applies it as `SetLink`. A typed address without a scheme gets `https://`, an email address `mailto:`, and `#name` points at a bookmark. With the caret or a selection inside a link, the command acts on the whole link, as in Word. A bar below a link shows its address, with Edit and Remove. ⌘-click, or Ctrl-click off a Mac, opens a link. Only `http`, `https` and `mailto` links open, in a new tab; others are shown, not opened.
- With the caret in a table cell, a table bar shows Row above, Row below, Column left, Column right, Remove row, Remove column and Remove table. As in Word's table menu, a new row or column copies the caret's. Next to a cell that spans columns, "left" is before its first column and "right" after its last. Remove column removes the first column the caret's cell covers.
- The Picture button, or pasting an image with no text, names the image with `mediaName` and puts it at the caret with `InsertPicture`, in place of the selection if there is one. A pasted image goes where it was pasted, even if the selection moves while the image is read. Its size is its pixel size at 96 dots per inch, made smaller to fit between the page's margins. PNG, JPEG and GIF images are taken; others are refused with a reason. Text pasted with an image, as Word copies it, is pasted as text.
- The Track changes button records typing and deleting as tracked changes, under the name beside it. Typing, Enter, Backspace, Delete, pasting, cutting and an IME's composed text each become one `ReplaceRange` with an author and a date. Backspace and Delete pass over what is deleted already, and at a paragraph's edge they take its paragraph mark. For ordinary text the browser supplies the deletion range, including when formatting splits an emoji across two runs. Deleting one's own insertion removes it. Typing on from one's own change keeps its date, so it stays one change. While tracking is on, the other commands above and Shift+Enter are refused.
- The Comment button and ⌘⌥M open a field below the selection, and Enter adds the comment as `AddComment`; Shift+Enter starts a new line in it. With nothing selected, the comment goes on the word at the caret, as in Word. The comment goes on what was selected when the field opened, wherever the selection moves while it is typed. If that text is removed meanwhile, the draft stays and the page says so. Each comment's balloon has a Reply button, which adds a reply as `ReplyToComment`, drawn in the same balloon. The author is the name beside Track changes. A comment is never a tracked change.
- The Update fields on open button asks Word to update the document's fields, such as a table of contents, when it opens the downloaded file, as `SetUpdateFields`. It is pressed while the document asks, and a click stops asking. It is never a tracked change, and undo takes it back on its own.
- A click on a form's content control fills it as `FillContentControl`. A check box ticks or clears. A text, list or date control opens a box under it with its value, and Enter or Fill puts the value in; Escape or a click elsewhere closes it. Locked, bound and nested controls are refused with the engine's reason, and so is any fill while Track changes is on.
- The Update fields button updates the body's fields as `UpdateFields`, as Word does when every field is selected and updated. Dates and times take the local time. Page numbers, the page count and page references, such as the page numbers in a table of contents, take the pages the editor laid out, numbered as each section numbers its pages. A locked field, and a field the engine cannot work out, keep their results. Header and footer page numbers need no update, since the layout draws them on each page. When no result would change, the page says so. An update is refused while Track changes is on, and undo takes it back on its own.

After a rebuild, Safari can keep running the previous build's modules from its cache, because the server sends no cache headers. Open the page in a private tab to load the new build. A failure while loading is shown at the bottom of the page.

## Editing together

The page always edits through a Yrs replica. With `?collab`, it shares that replica's state live through the collaboration server.

Open the server's address in two browsers. Both edit `complex-mixed.docx`, and each sees the other's edits as they are made. The bar lists who is connected; type your name in the box beside it. Each person's caret and selection show in the others' views, with their name.

- `?collab&fixture=real-world/python-docx.docx` shares another fixture.
- Choosing a file uploads it, and the page moves to an address others can open.
- Undo and redo take back only your own changes, and leave everyone else's.
- A page in a background tab or a minimized window applies the others' edits when it is shown again.
- The server keeps uploads and edits in memory. After a restart, an open tab can restore a bundled fixture’s edits when it reconnects. An uploaded file’s room loses its base DOCX, so its address stops working. Anyone with that editor still open can use **Download DOCX** before closing the tab. Uploading the edited download starts a new history.
- Anyone who can reach the port can read and edit every document on it.

## What the pages show

| Content | Shown |
| --- | --- |
| Paragraphs | Spacing, indents, alignment, line spacing, tab stops with their alignment and leaders, list markers, small capitals and capitals |
| Fonts | Calibri, Cambria, Arial, Times New Roman and Courier New use open fonts of the same widths. Other families use installed fonts when available. Open loads the families used by content and styles, including Tinos for review labels (32 files, 0.78 MB on a cold page). All five bundled families together are 132 WOFF2 files (2.8 MB). Successful loads are shared across opens. |
| Pages | Size, orientation and margins per section; columns; how each section starts; first-page, even and odd headers and footers; page numbers, page counts and section page counts |
| Keep rules | Keep with next, keep lines together, widow and orphan control, page break before |
| Tables | Grid widths, borders and fills as the engine resolves them, cell margins, vertical alignment, merged cells, header rows repeated on each page, rows split between lines |
| Pictures | Inline and anchored, with text wrapped around them or not, in front of or behind the text: PNG, JPEG, GIF, BMP, WebP and SVG |
| Notes | Footnotes at the foot of the page that refers to them, endnotes after the body |
| Tracked changes | Insertions and moved text on the line; deletions, moves and formatting changes in balloons beside the page, as Word shows them; change bars |
| Comments | In balloons beside the page, with the text they cover shaded and each reply in the balloon of the comment it answers |

A click on a balloon puts the caret at its change, and a move's arrow goes to the move's other end.

Not shown: conditional table formatting and balanced columns. A picture the browser cannot show, such as EMF or WMF, is a labelled box. The rendering rules are in the [spec](../../docs/pipelines/00004-docx-engine-demo/spec.md).

## Checks

`moon check --all` runs these five, after building the editor package:

| Task | What it runs |
| --- | --- |
| `editor:test` | The layout with a fixed measurer, and where the editor places a typed change |
| `editor:types` | That the package's `wire.generated.ts` is what the Rust types generate |
| `demo:test` | How the Word comparison scores lines |
| `demo:fixtures` | Every fixture through the WebAssembly module in Node: open, inspect, edit, export, validate, re-open and lay out |
| `demo:collab` | The collaboration server and its replicas in Node, with an upload and a restart |

The others run on request:

- `moon run demo:browser` edits in the built demo in headless Chrome, a feature at a time, alone and in two shared tabs. It needs Google Chrome at its usual place on a Mac, and takes about a minute. `node scripts/check-browser.mjs range table` runs only the features named.
- `scripts/check-with-python-docx.sh` edits every fixture and reads the result with python-docx. It needs uv.
- To compare the layout with Word, run `node scripts/compare-with-word.mjs`. It needs Microsoft Word, Google Chrome and poppler (`pdftotext`, `pdfinfo`), and a built demo. The first run makes Word ask for access to `target/word-compare`. Choose that folder in Word's "Grant File Access" dialog, then run it again. It exits non-zero when a fixture falls outside the tolerance: the same page count as Word, and 95% of lines broken the same way.
- `node scripts/bench-editor.mjs` times the editor in headless Chrome. Its header says how to make the large documents it reads.
