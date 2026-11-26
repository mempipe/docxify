import { formatNumber as engineNumber } from "./pkg/docx_engine_wasm.js";

export function formatNumber(n: number, numberFormat = "decimal"): string {
  return engineNumber(n, numberFormat);
}
