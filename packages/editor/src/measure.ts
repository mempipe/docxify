// Text measurement, behind an interface: the layout never touches the DOM,
// and tests measure with fixed advances. Every length is in points.

/** A face the layout measures with: the family is the one actually used, after substitution. */
export interface Font {
  family: string;
  size: number;
  bold: boolean;
  italic: boolean;
  smallCaps: boolean;
}

/** A font's single line, split at the baseline. */
export interface Metrics {
  ascent: number;
  descent: number;
}

export interface Measurer {
  width(text: string, font: Font): number;
  metrics(font: Font): Metrics;
}

export const cssFont = (f: Font) =>
  `${f.italic ? "italic " : ""}${f.smallCaps ? "small-caps " : ""}${f.bold ? 700 : 400} ${f.size}pt "${f.family}"`;

const PX_TO_PT = 72 / 96;

/**
 * Widths from canvas `measureText`, which uses the same shaping as the page,
 * so a word drawn at a measured position fills exactly its measured width.
 * Vertical metrics come from one probe per face: canvas does not expose the
 * line gap, which is part of Word's single line.
 */
export class CanvasMeasurer implements Measurer {
  private readonly context: CanvasRenderingContext2D;
  private widths = new Map<string, Map<string, number>>();
  private faces = new Map<string, Metrics>();
  /** By font object: a paragraph keeps its fonts while unchanged, so each layout asks again with the same ones. */
  private css = new WeakMap<Font, string>();
  private sized = new WeakMap<Font, Metrics>();

  constructor() {
    this.context = document.createElement("canvas").getContext("2d")!;
  }

  width(text: string, font: Font): number {
    let css = this.css.get(font);
    if (css === undefined) this.css.set(font, (css = cssFont(font)));
    let cache = this.widths.get(css);
    if (!cache) this.widths.set(css, (cache = new Map()));
    let width = cache.get(text);
    if (width === undefined) {
      if (this.context.font !== css) this.context.font = css;
      width = this.context.measureText(text).width * PX_TO_PT;
      cache.set(text, width);
    }
    return width;
  }

  metrics(font: Font): Metrics {
    let sized = this.sized.get(font);
    if (!sized) this.sized.set(font, (sized = this.measured(font)));
    return sized;
  }

  private measured(font: Font): Metrics {
    const face = cssFont({ ...font, size: 100 });
    let metrics = this.faces.get(face);
    if (!metrics) {
      const probe = document.createElement("div");
      probe.style.cssText = `position:absolute;visibility:hidden;left:0;top:0;white-space:nowrap;line-height:normal;font:${face}`;
      const baseline = document.createElement("span");
      baseline.style.cssText = "display:inline-block;width:0;height:0;vertical-align:baseline";
      probe.append("Hg", baseline);
      document.body.append(probe);
      const height = probe.getBoundingClientRect().height;
      const ascent = baseline.getBoundingClientRect().top - probe.getBoundingClientRect().top;
      probe.remove();
      metrics = { ascent: ascent * PX_TO_PT, descent: (height - ascent) * PX_TO_PT };
      this.faces.set(face, metrics);
    }
    const scale = font.size / 100;
    return { ascent: metrics.ascent * scale, descent: metrics.descent * scale };
  }
}

/** Every character half an em wide, for checks that must not depend on a font. */
export class FixedMeasurer implements Measurer {
  width(text: string, font: Font): number {
    return [...text].length * font.size * 0.5;
  }

  metrics(font: Font): Metrics {
    // The descent is the rest of the em, so the two add up to exactly its size.
    const ascent = font.size * 0.8;
    return { ascent, descent: font.size - ascent };
  }
}
