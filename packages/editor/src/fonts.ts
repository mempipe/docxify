// Which face draws each family a document asks for. Measuring and painting
// both go through this one table.
//
// Word's core families map to open fonts with the same advance widths, so a
// line breaks where it breaks in Word, on every machine. Any other family is
// used if the browser has it, and otherwise falls back.

/** Families with the same metrics as Word's, as Fontsource packages. */
const COMPATIBLE: Record<string, { family: string; pkg: string }> = {};
for (const [pkg, family, names] of [
  ["carlito", "Carlito", ["Calibri"]],
  ["caladea", "Caladea", ["Cambria"]],
  ["arimo", "Arimo", ["Arial", "Helvetica", "Liberation Sans"]],
  ["tinos", "Tinos", ["Times New Roman", "Times", "Liberation Serif"]],
  ["cousine", "Cousine", ["Courier New", "Courier", "Liberation Mono"]],
] as const) {
  for (const name of [...names, family]) COMPATIBLE[name.toLowerCase()] = { family, pkg };
}

const FALLBACK = COMPATIBLE.arial;
const FACES = ["400", "400-italic", "700", "700-italic"];

export class FontResolver {
  private resolved = new Map<string, string>();
  private readonly available: (family: string) => boolean;

  /** Whether the browser can draw a family it was not given. */
  constructor(available: (family: string) => boolean = () => false) {
    this.available = available;
  }

  /** The family to measure and paint `requested` with. */
  resolve(requested: string): string {
    const key = requested.toLowerCase();
    let found = this.resolved.get(key);
    if (!found) {
      const compatible = COMPATIBLE[key];
      found = compatible?.family ?? (this.available(requested) ? requested : FALLBACK.family);
      this.resolved.set(key, found);
    }
    return found;
  }

  /** Wait until the families this document uses are ready to measure. */
  async load(): Promise<boolean> {
    const loads = await Promise.allSettled([...this.resolved.values()].map(used => {
      const bundled = COMPATIBLE[used.toLowerCase()];
      if (!bundled) return;
      let ready = bundledFonts.get(bundled.pkg);
      if (!ready) {
        ready = loadFamily(bundled.family, bundled.pkg);
        bundledFonts.set(bundled.pkg, ready);
      }
      return ready;
    }));
    return loads.every(load => load.status === "fulfilled");
  }
}

// Stylesheets and faces belong to the page. Concurrent and later opens share the same load.
const bundledFonts = new Map<string, Promise<void>>();

async function loadFamily(family: string, pkg: string) {
  const links = FACES.map(face => Object.assign(document.createElement("link"), {
    rel: "stylesheet",
    href: `./node_modules/@fontsource/${pkg}/${face}.css`,
  }));
  try {
    await Promise.all(links.map(link => new Promise<void>((resolve, reject) => {
      link.onload = () => resolve();
      link.onerror = reject;
      document.head.append(link);
    })));
    // Each script subset is a separate face, including ones absent from the opening text.
    await Promise.all([...document.fonts].filter(face => face.family === family).map(face => face.load()));
  } catch (error) {
    // The browser retains failed faces on their stylesheet. A later open needs fresh faces.
    for (const link of links) link.remove();
    bundledFonts.delete(pkg);
    throw error;
  }
}

/**
 * Whether the browser draws `family` itself: text set in it measures
 * differently from both generic families it would otherwise fall back to.
 */
export function installed(): (family: string) => boolean {
  const context = document.createElement("canvas").getContext("2d")!;
  const sample = "mmmmmmmmmmlli1WQ@#";
  const width = (font: string) => {
    context.font = `72px ${font}`;
    return context.measureText(sample).width;
  };
  const generic = ["monospace", "serif"].map((g) => [g, width(g)] as const);
  return (family) => generic.some(([g, w]) => width(`"${family.replace(/"/g, "")}", ${g}`) !== w);
}
