// The same engine in a browser: open a DOCX, edit it on its laid-out pages
// (surface.ts, over the ProseMirror views of editor.ts), and download the
// exported DOCX.
// Every document is edited through a replica of its editing state
// (collab.ts). With `?collab`, served by collab-server.ts, that state is
// shared live with everyone who opens the same address. `fixture=` or
// `upload=` names the document.

import { alone, connect, guest, type Presence, type ReplicaSession, type User } from "@mempipe/docxify-editor/collab";
import { builtIn, type EditorEvents, FORMATTING, LISTS, resolutionNotes, TABLE_EDITS } from "@mempipe/docxify-editor/editor";
import { atomText } from "@mempipe/docxify-editor/model";
import { loadDocxEngine } from "@mempipe/docxify-editor/engine";
import { type HeldSelection, Surface, SURFACE_CSS } from "@mempipe/docxify-editor/surface";
import type { Fill, Inspection, InspectNode, NodeId } from "@mempipe/docxify-editor/types";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const status = (value: unknown) => ($("status").textContent = JSON.stringify(value, null, 2));

const style = document.createElement("style");
style.textContent = SURFACE_CSS;
document.head.append(style);

const params = new URLSearchParams(location.search);
const engine = await loadDocxEngine();
const collab = params.has("collab");
let session: ReplicaSession | undefined;
let presence: Presence = { status: "connecting", people: [] };
let surface: Surface | undefined;
let name = "document.docx";
let preserved = "";

// For poking at the engine from the browser console.
Object.assign(globalThis, {
  docxDemo: {
    engine,
    get surface() {
      return surface;
    },
    get session() {
      return session;
    },
  },
});

function summarize() {
  if (!session) return;
  $("updatefields").setAttribute("aria-pressed", String(session.settings().update_fields));
  const notes = resolutionNotes(session.replica.resolutions());
  $("summary").textContent = notes.length ? `${preserved}. ${notes.join(" ")}` : preserved;
  if (!collab) return;
  const { status, people } = presence;
  $("people").replaceChildren(
    ...(status === "connected"
      ? people.map(({ name, color, self }) => {
          const person = document.createElement("span");
          person.className = "person";
          person.style.setProperty("--color", color);
          person.textContent = self ? `${name} (you)` : name;
          return person;
        })
      : [status]),
  );
}

const NAME = "docx-engine-demo.user";

/** This browser's name and colour, kept between visits. */
function me(): User {
  try {
    const saved = JSON.parse(localStorage.getItem(NAME) ?? "null") as User | null;
    if (saved?.name && saved.color) return saved;
  } catch {}
  const fresh = guest();
  try {
    localStorage.setItem(NAME, JSON.stringify(fresh));
  } catch {}
  return fresh;
}

let user = me();
/** Shares a changed name with the others editing, once joined. */
let rename: ((user: User) => void) | undefined;
let tracking = false;

const author = $<HTMLInputElement>("name");
author.value = user.name;
author.addEventListener("change", () => {
  // A new object: awareness compares the old state with the new one, and a
  // mutated object would compare equal, so this tab would not see its own name.
  user = { ...user, name: author.value.trim() || user.name };
  author.value = user.name;
  try {
    localStorage.setItem(NAME, JSON.stringify(user));
  } catch {}
  rename?.(user);
  if (session && tracking) session.tracking = user.name;
});

// Changes are tracked under the name beside the button.
$("track").addEventListener("mousedown", (event) => event.preventDefault());
$("track").addEventListener("click", () => {
  tracking = !tracking;
  $("track").setAttribute("aria-pressed", String(tracking));
  if (session) session.tracking = tracking ? user.name : null;
});

let noticeTimer: ReturnType<typeof setTimeout> | undefined;
function notice(message: string) {
  $("notice").textContent = message;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => ($("notice").textContent = ""), 5000);
}

const events: EditorEvents = {
  refused: notice,
  linking: () => editLink(),
  commenting: () => editComment(surface?.caretRect() ?? null, null),
  formatting: (shown, style, list, link, inTable) => {
    $("tablebar").hidden = !inTable;
    for (const property of FORMATTING) $(property).setAttribute("aria-pressed", String(shown[property]));
    for (const kind of LISTS) $(kind).setAttribute("aria-pressed", String(list === kind));
    $("link").setAttribute("aria-pressed", String(link !== null));
    showLink(link);
    const normal = surface && builtIn(surface.inspection.styles, 0);
    // An empty value is the blank choice: the paragraphs differ.
    $<HTMLSelectElement>("style").value = style === null ? "" : style || (normal?.id ?? "");
  },
};

/** The document's paragraph styles, the default first, then by name as Word shows them. */
function listStyles(inspection: Inspection) {
  const shown = (name: string) => name.charAt(0).toUpperCase() + name.slice(1);
  const styles = inspection.styles
    .filter((s) => s.kind === "paragraph")
    .map((s) => ({ id: s.id, label: shown(s.name || s.id), first: !!s.is_default }))
    .sort((a, b) => Number(b.first) - Number(a.first) || a.label.localeCompare(b.label));
  const blank = Object.assign(document.createElement("option"), { value: "", hidden: true });
  $("style").replaceChildren(
    blank,
    ...styles.map(({ id, label }) => Object.assign(document.createElement("option"), { value: id, textContent: label })),
  );
}

$<HTMLSelectElement>("style").addEventListener("change", (event) => {
  const { value } = event.target as HTMLSelectElement;
  if (value) surface?.style(value);
});

for (const property of FORMATTING) {
  // Pressing the button would take focus, and the selection, from the editor.
  $(property).addEventListener("mousedown", (event) => event.preventDefault());
  $(property).addEventListener("click", () => surface?.format(property));
}

for (const kind of LISTS) {
  $(kind).addEventListener("mousedown", (event) => event.preventDefault());
  $(kind).addEventListener("click", () => surface?.list(kind));
}

$("link").addEventListener("mousedown", (event) => event.preventDefault());
$("link").addEventListener("click", () => surface?.startLink());

$("comment").addEventListener("mousedown", (event) => event.preventDefault());
$("comment").addEventListener("click", () => surface?.startComment());

$("updatefields").addEventListener("mousedown", (event) => event.preventDefault());
$("updatefields").addEventListener("click", () => {
  if (session) surface?.updateFields(!session.settings().update_fields);
});

$("refreshfields").addEventListener("mousedown", (event) => event.preventDefault());
$("refreshfields").addEventListener("click", () => surface?.updateFieldResults());

$("picture").addEventListener("mousedown", (event) => event.preventDefault());
$("picture").addEventListener("click", () => $<HTMLInputElement>("pictures").click());
$<HTMLInputElement>("pictures").addEventListener("change", (event) => {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  // Choosing the same file again fires no change otherwise.
  input.value = "";
  if (!file) return;
  // The picker took the keyboard. The read that follows must not take it back later.
  surface?.focus();
  void surface?.picture(file);
});

for (const edit of TABLE_EDITS) {
  $(edit).addEventListener("mousedown", (event) => event.preventDefault());
  $(edit).addEventListener("click", () => surface?.table(edit));
}

/**
 * Where a document's link may lead from this page: web and email
 * addresses. Anything else, such as `javascript:`, could run in the page,
 * so it never opens.
 */
function openable(href: string): string | null {
  try {
    const url = new URL(href);
    return url.protocol === "http:" || url.protocol === "https:" || url.protocol === "mailto:" ? url.href : null;
  } catch {
    return null;
  }
}

function follow(href: string) {
  const url = openable(href);
  if (url) window.open(url, "_blank", "noopener,noreferrer");
  else notice(href.startsWith("#") ? "Links to places in this document do not open yet." : "Only web and email links open from here.");
}

/**
 * What someone typed as a link, as the document will hold it: an address
 * without a scheme is a web address, or an email one if it looks like one,
 * and `#` names a bookmark. Null for anything a link should not hold.
 */
function linkTarget(typed: string): string | null {
  const text = typed.trim();
  if (text.startsWith("#")) return /^#\S+$/.test(text) ? text : null;
  let target = `https://${text}`;
  if (/^(https?|ftp):\/\//i.test(text) || /^(mailto|tel):/i.test(text)) target = text;
  else if (/^[^\s@/]+@[^\s@/]+\.[^\s@/]+$/.test(text)) target = `mailto:${text}`;
  try {
    new URL(target);
    return text && !/\s/.test(text) ? target : null;
  } catch {
    return null;
  }
}

const linkbar = $("linkbar");
const linkform = $<HTMLFormElement>("linkform");
const linkurl = $<HTMLInputElement>("linkurl");
const linkview = $("linkview");
const linkopen = $<HTMLAnchorElement>("linkopen");
/** Where the link the caret is in points, or null outside one. */
let shownLink: string | null = null;
/** What the address being asked for is for, while the form is open. */
let editingLink: HeldSelection | null = null;

/** Puts the link bar under the caret, inside the window. */
function placeLinkbar() {
  const caret = surface?.caretRect();
  if (!caret) return void (linkbar.hidden = true);
  const left = Math.max(16, Math.min(caret.left, innerWidth - linkbar.offsetWidth - 16));
  linkbar.style.left = `${left + scrollX}px`;
  linkbar.style.top = `${caret.bottom + scrollY + 6}px`;
}

/** Shows where the link the caret is in points, with Edit and Remove, or nothing outside a link. */
function showLink(href: string | null) {
  shownLink = href;
  if (editingLink) return;
  linkbar.hidden = href === null;
  if (href === null) return;
  linkform.hidden = true;
  linkview.hidden = false;
  const url = openable(href);
  linkopen.textContent = href || "(no address)";
  if (url) linkopen.href = url;
  else linkopen.removeAttribute("href");
  linkopen.title = url ? "Open in a new tab (or ⌘-click the link)" : "This link does not open from here";
  requestAnimationFrame(placeLinkbar);
}

/** Asks for an address for the selection, or for the link the caret is in. */
function editLink() {
  editingLink = surface?.holdLink() ?? null;
  if (!editingLink) return;
  linkbar.hidden = false;
  linkview.hidden = true;
  linkform.hidden = false;
  linkurl.value = shownLink ?? "";
  placeLinkbar();
  linkurl.focus();
  linkurl.select();
}

function stopEditingLink() {
  editingLink = null;
  linkform.hidden = true;
  linkbar.hidden = true;
}

linkform.addEventListener("submit", (event) => {
  event.preventDefault();
  const target = linkTarget(linkurl.value);
  if (!target) return notice("That is not a web or email address.");
  // A refusal leaves the form open, so the address can still be read and copied.
  if (editingLink && surface?.link(target, editingLink)) stopEditingLink();
});
// Pressing Apply would blur the address first, which closes the form.
$("linkapply").addEventListener("mousedown", (event) => event.preventDefault());
linkurl.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  event.preventDefault();
  stopEditingLink();
  surface?.focus();
});
linkurl.addEventListener("blur", stopEditingLink);
for (const id of ["linkopen", "linkedit", "linkremove"]) {
  // Keeps the caret, and the link it is in, while the bar is used.
  $(id).addEventListener("mousedown", (event) => event.preventDefault());
}
$("linkedit").addEventListener("click", editLink);
$("linkremove").addEventListener("click", () => surface?.link(null));

const commentbar = $<HTMLFormElement>("commentbar");
const commenttext = $<HTMLTextAreaElement>("commenttext");
/** A new comment holds a selection. A reply holds its thread's stable identity. */
let draftTarget: HeldSelection | string | null = null;

/** Asks for a comment under `at`, the caret, or for a reply to `comment` under its Reply button. */
function editComment(at: DOMRect | null, comment: string | null) {
  const thread = comment === null ? null : session?.replica.commentIdentities()[comment];
  if (thread === undefined) return notice("This comment was removed.");
  const target = thread ?? surface?.holdComment();
  if (!target) return;
  const previousThread = typeof draftTarget === "string" ? draftTarget : null;
  if (!draftTarget || previousThread !== thread) commenttext.value = "";
  draftTarget = target;
  commenttext.placeholder = comment === null ? "Add a comment (Shift+Enter for a new line)" : "Reply";
  commentbar.hidden = false;
  const below = at ?? new DOMRect(16, $("view").getBoundingClientRect().top, 0, 0);
  const left = Math.max(16, Math.min(below.left, innerWidth - commentbar.offsetWidth - 16));
  commentbar.style.left = `${left + scrollX}px`;
  commentbar.style.top = `${below.bottom + scrollY + 6}px`;
  commenttext.focus();
}

function closeComment() {
  draftTarget = null;
  commentbar.hidden = true;
}

commentbar.addEventListener("submit", (event) => {
  event.preventDefault();
  const text = commenttext.value.trim();
  if (!text) return notice("Type the comment first.");
  if (!draftTarget || !surface || !session) return;
  const by = { author: user.name, date: new Date().toISOString().replace(/\.\d+Z$/, "Z") };
  let posted: boolean;
  if (typeof draftTarget === "string") {
    const number = Object.entries(session.replica.commentIdentities()).find(([, id]) => id === draftTarget)?.[0];
    if (number === undefined) return notice("This comment was removed. Your draft is still here.");
    posted = surface.reply(number, text, by);
  } else {
    posted = surface.comment(text, by, draftTarget);
  }
  if (posted) closeComment();
  else commenttext.focus();
});
commenttext.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
    event.preventDefault();
    commentbar.requestSubmit();
  } else if (event.key === "Escape") {
    event.preventDefault();
    closeComment();
    surface?.focus();
  }
});
// A comment half typed stays when the field loses focus; an empty field closes.
commenttext.addEventListener("blur", () => {
  if (!commenttext.value.trim()) closeComment();
});

const fillbar = $<HTMLFormElement>("fillbar");
/** The control the fill box fills, its field, and the fill the field's value makes. */
let filling: { control: NodeId; field: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement; fill: (value: string) => Fill } | null =
  null;

/** Ticks or clears a check box, or asks for a control's value under the text clicked. */
function fillControl(node: InspectNode, at: DOMRect) {
  if (node.type !== "content_control") return;
  const { control } = node;
  if (control.type === "checkbox") return surface?.fill(node.id, { checked: !control.checked });
  let field: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;
  let fill: (value: string) => Fill;
  switch (control.type) {
    case "text":
    case "rich_text":
      field = document.createElement(control.type === "rich_text" || control.multi_line ? "textarea" : "input");
      field.value = node.placeholder ? "" : atomText(node);
      fill = (text) => ({ text });
      break;
    case "drop_down":
    case "combo_box": {
      const select = document.createElement("select");
      for (const item of control.items) select.add(new Option(item.display_text || item.value, item.value));
      select.value = control.last_value ?? "";
      field = select;
      fill = (item) => ({ item });
      break;
    }
    case "date": {
      const input = document.createElement("input");
      input.type = "date";
      input.value = control.full_date?.slice(0, 10) ?? "";
      field = input;
      fill = (date) => ({ date });
      break;
    }
    default:
      return;
  }
  field.setAttribute("aria-label", node.alias ?? "Form field");
  $("fillfield").replaceChildren(field);
  filling = { control: node.id, field, fill };
  fillbar.hidden = false;
  const left = Math.max(16, Math.min(at.left, innerWidth - fillbar.offsetWidth - 16));
  fillbar.style.left = `${left + scrollX}px`;
  fillbar.style.top = `${at.bottom + scrollY + 6}px`;
  field.focus();
}

function stopFilling() {
  filling = null;
  fillbar.hidden = true;
}

fillbar.addEventListener("submit", (event) => {
  event.preventDefault();
  if (!filling) return;
  const { control, field, fill } = filling;
  if (surface?.fill(control, fill(field.value))) stopFilling();
});
fillbar.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    stopFilling();
    surface?.focus();
  } else if (event.key === "Enter" && event.target instanceof HTMLTextAreaElement && !event.shiftKey && !event.isComposing) {
    event.preventDefault();
    fillbar.requestSubmit();
  }
});
fillbar.addEventListener("focusout", stopFilling);
// Pressing Fill would blur the field first, which closes the box.
$("fillapply").addEventListener("mousedown", (event) => event.preventDefault());

async function open(bytes: Uint8Array, filename: string) {
  await show(alone(engine, bytes, events, summarize), filename);
}

async function fetchDocument(file: string) {
  const response = await fetch(`../${file}`);
  if (!response.ok) throw new Error(`${file}: ${response.status} ${await response.text()}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function join(file: string) {
  const base = await fetchDocument(file);
  // A room on the second path is another room, with another shared state.
  const room = `${engine.roomPrefix}${file}`;
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  const connection = await connect(engine, {
    url: `${scheme}://${location.host}/collab`,
    room,
    base,
    user,
    events,
    changed: summarize,
    presence: (now) => {
      presence = now;
      summarize();
    },
  });
  rename = (user) => connection.provider.setAwarenessField("user", user);
  $("hint").textContent =
    "Anyone who opens this page's address edits the same document. Clicking a form field fills it in. Track changes records your typing and deleting under your name. ⌘Z or Ctrl+Z undoes your own changes.";
  try {
    await show(connection.session, room.split("/").pop()!);
  } catch (error) {
    connection.provider.destroy();
    throw error;
  }
}

async function show(next: ReplicaSession, filename: string) {
  let opened: Surface;
  try {
    opened = await Surface.open(next.replica, next, notice);
  } catch (error) {
    next.replica.free();
    throw error;
  }
  surface?.destroy();
  session?.replica.free();
  stopFilling();
  stopEditingLink();
  closeComment();
  session = next;
  session.tracking = tracking ? user.name : null;
  name = filename;
  surface = opened;
  $("view").replaceChildren(opened.root);
  surface.onFollow = follow;
  surface.onReply = (comment, at) => editComment(at, comment);
  surface.onControl = fillControl;
  const { inspection } = surface;
  listStyles(inspection);
  preserved = `${inspection.unsupported.reduce((n, u) => n + u.count, 0)} preserved elements not editable; ${inspection.diagnostics.length} diagnostics`;
  summarize();
}

$("download").addEventListener("click", () => {
  if (!session) return;
  const blob = new Blob([session.replica.export() as BlobPart], {
    type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  });
  const a = Object.assign(document.createElement("a"), {
    href: URL.createObjectURL(blob),
    download: name.replace(/\.docx$/i, ".edited.docx"),
  });
  a.click();
  URL.revokeObjectURL(a.href);
});

const picker = $<HTMLInputElement>("file");
picker.addEventListener("change", async () => {
  const file = picker.files?.[0];
  if (!file || picker.disabled) return;
  // Selecting this same file again must fire change, including after a failed read.
  picker.value = "";
  picker.disabled = true;
  try {
    if (!collab) return await open(new Uint8Array(await file.arrayBuffer()), file.name);
    // A shared document lives on the server, under a name others can open.
    const response = await fetch("/upload", { method: "POST", body: file });
    if (!response.ok) return notice(await response.text());
    const { upload } = await response.json();
    location.search = `?collab&upload=${upload}`;
  } catch (error) {
    notice(`Could not open ${file.name}. ${String(error)}`);
  } finally {
    picker.disabled = false;
  }
});

const fixture = params.get("fixture") ?? "generated/complex-mixed.docx";
try {
  if (collab) {
    const upload = params.get("upload");
    await join(upload ? `upload/${upload}.docx` : `fixtures/${fixture}`);
  } else {
    await open(await fetchDocument(`fixtures/${fixture}`), fixture.split("/").pop()!);
  }
} catch (error) {
  // No pages are shown, so the reason takes their place until a file is chosen.
  $("view").textContent = `Could not open the document. ${String(error)}`;
} finally {
  picker.disabled = false;
}
