// The wire declarations come from serde. Layout uses additional guarantees
// from the inspector, such as resolved formatting on every paragraph and run.
import type * as Wire from "./wire.generated.ts";
export type { Command, Control, Diagnostic, Fill, Format, ListItem, NodeId, Pages, Point,
  Resolution, Settings, Splice, TextRange, Track, ValidationReport } from "./wire.generated.ts";

// Optional properties on the serialized inspection are omitted when absent.
type Present<T> = T extends object ? { [K in keyof T]: Present<Exclude<T[K], null>> } : T;
export type RunProps = Present<Wire.RunProps>;
export type PageField = Present<Wire.PageField>;
export type EffectiveRun = RunProps & Required<Pick<RunProps, "font" | "size_half_points">>;
export type Tracked = Present<Wire.Tracked>;
export type Style = Omit<Present<Wire.Style>, "paragraph">;
export type Support = "editable" | "read_only" | "opaque";
export type Border = Present<Wire.Border>;
export type Edges<T> = { [K in keyof Wire.Edges]?: T };
export type StoryRef = Present<Wire.StoryRef> & { kind: "default" | "first" | "even" };
export type Section = Present<Wire.SectionInfo> & { headers?: StoryRef[]; footers?: StoryRef[] };
export type Placement = Present<Wire.Placement>;
export type Level = Present<Wire.Level> & { suffix: "tab" | "space" | "nothing"; justification: "left" | "center" | "right" };
export type TabStop = Present<Wire.TabStop> & {
  align: "left" | "center" | "right" | "decimal" | "bar";
  leader?: "dot" | "hyphen" | "underscore" | "heavy" | "middle_dot";
};
export type EffectiveParagraph = Omit<Present<Wire.EffectiveParagraph>, "tabs"> & {
  alignment: "left" | "center" | "right" | "justify";
  spacing: Wire.Spacing & { line_rule: "auto" | "exact" | "at_least" };
  tabs: TabStop[];
};

type Data = Present<Wire.NodeData>;
type Kind<K extends Data["type"]> = Extract<Data, { type: K }>;
type Formatting = Present<Wire.NodeFormatting>;
type Paragraph = Kind<"paragraph"> & {
  paragraph_text: string; effective: EffectiveParagraph; mark: EffectiveRun;
  section?: Section; list?: Present<Wire.ListView> & { definition?: Level };
};
type Table = Kind<"table"> & Extract<Formatting, { cells: object }> & { alignment: "left" | "center" | "right" };
type Cell = Kind<"cell"> & Extract<Formatting, { borders: object }> & {
  margins_twips: Required<Edges<number>>; vertical_align: "top" | "center" | "bottom";
};
type Row = Kind<"row"> & { height?: Wire.RowHeight & { rule: "at_least" | "exact" } };
type Base = Present<Omit<Wire.NodeFields, "children">> & { support: Support; children?: InspectNode[] };
export type InspectNode = Base & (
  | Paragraph | Table | Cell | Row | (Kind<"run"> & { effective: EffectiveRun }) | (Kind<"section"> & Section)
  | Exclude<Data, { type: "paragraph" | "run" | "table" | "cell" | "row" | "section" }>
);
export type Inspection = Present<Omit<Wire.Inspection, "stories" | "styles">> & {
  stories: InspectNode[]; styles: Style[];
};
