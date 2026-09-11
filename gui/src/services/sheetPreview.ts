// xlsx 预览服务:SheetJS(vendor 本地 tgz,见 gui/vendor)解析 + 纯 DOM
// 建表。不用 innerHTML/HTML 模板:单元格文本一律 textContent 写入,
// 恶意工作簿里的 HTML/公式文本不会成为标记。
import * as XLSX from "xlsx";
import type { WorkBook } from "xlsx";

/** 单 sheet 渲染上限:标书级大表(C_big 审计样本)全量入 DOM
 * 会拖垮 webview,超限部分截断并提示"用系统程序打开"看全量。 */
export const MAX_SHEET_ROWS = 500;
export const MAX_SHEET_COLS = 100;

/** 原型链危险键:工作表名来自文件本身,查找 wb.Sheets 时拒绝这类名字
 * (防御纵深;SheetJS 0.20.2+ 已修解析期原型污染,见官方 advisories)。 */
const UNSAFE_SHEET_NAME = /^(__proto__|constructor|prototype)$/;

/** 解析一次并缓存:同一文件的 listSheets/renderSheet 共享 workbook,
 * 避免重复解析(大文件内存翻倍)。组件卸载时调用 releaseWorkbook。 */
let cachedWb: WorkBook | null = null;

export function parseWorkbook(bytes: Uint8Array): string[] {
  cachedWb = XLSX.read(bytes, { type: "array" });
  return cachedWb.SheetNames.filter(
    (name) => !UNSAFE_SHEET_NAME.test(name),
  );
}

export function releaseWorkbook(): void {
  cachedWb = null;
}

export interface SheetRenderResult {
  rows: number;
  cols: number;
  truncated: boolean;
}

/** 把指定 sheet 的前 MAX 行/列渲染为 DOM 表格追加到 container。 */
export function renderSheet(
  sheetName: string,
  container: HTMLElement,
): SheetRenderResult {
  if (UNSAFE_SHEET_NAME.test(sheetName)) {
    throw new Error("非法工作表名");
  }
  const wb = cachedWb;
  if (!wb) throw new Error("工作簿未解析");
  const ws = wb.Sheets[sheetName];
  if (!ws) throw new Error(`工作表 ${sheetName} 不存在`);
  const ref = ws["!ref"] ?? "A1";
  const total = XLSX.utils.decode_range(ref);
  const endRow = Math.min(total.e.r, MAX_SHEET_ROWS - 1);
  const endCol = Math.min(total.e.c, MAX_SHEET_COLS - 1);
  // range 截断在解析侧完成,避免为超大表物化全量数组
  const grid: string[][] = XLSX.utils.sheet_to_json(ws, {
    header: 1,
    raw: false,
    defval: "",
    blankrows: true,
    range: { s: { r: total.s.r, c: total.s.c }, e: { r: endRow, c: endCol } },
  });

  const table = document.createElement("table");
  table.className = "sheet-table";
  for (const row of grid) {
    const tr = document.createElement("tr");
    for (const cell of row) {
      const td = document.createElement("td");
      td.textContent = cell;
      tr.appendChild(td);
    }
    table.appendChild(tr);
  }
  container.replaceChildren(table);

  return {
    rows: Math.min(total.e.r - total.s.r + 1, MAX_SHEET_ROWS),
    cols: Math.min(total.e.c - total.s.c + 1, MAX_SHEET_COLS),
    truncated: total.e.r > endRow || total.e.c > endCol,
  };
}
