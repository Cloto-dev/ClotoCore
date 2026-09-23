import type { ModuleInfo, PanelGroup } from '../types';

/**
 * A connector that declares several panels has them shown as the pages of one
 * view, in the order it declared them (MGP_CONNECTOR.md §4.1). The kernel lists
 * every panel as its own row, sorted by id, and says on each which connector it
 * belongs to and where it stands in that connector's declaration; the grouping
 * and the order are read from there, never from the id.
 *
 * Each page stays its own panel — its declared requests, its writes and the
 * operator's consent to them are not shared with the other pages.
 */

/** The pages of one connector's view, in declared order. */
export interface PanelPages {
  group: PanelGroup;
  pages: ModuleInfo[];
  /** Where the page asked about stands in `pages`. */
  index: number;
}

/** The usable panels of `connector`, in declared order. A rejected panel is not a page. */
function pagesOfConnector(modules: ModuleInfo[], connector: string): ModuleInfo[] {
  return modules
    .filter((m) => !m.error && m.connector?.id === connector)
    .sort((a, b) => (a.connector?.position ?? 0) - (b.connector?.position ?? 0));
}

/**
 * The view `id` is a page of, or `null` when it is a view of its own: a module
 * placed by hand, a connector with a single usable panel, or an id the listing
 * does not have.
 */
export function pagesOf(modules: ModuleInfo[], id: string): PanelPages | null {
  const entry = modules.find((m) => m.id === id && !m.error);
  const group = entry?.connector;
  if (!group) return null;
  const pages = pagesOfConnector(modules, group.id);
  if (pages.length < 2) return null;
  return { group, pages, index: pages.findIndex((m) => m.id === id) };
}

/** One entry of the sidebar: a module, or a connector's view of several pages. */
export interface SidebarModule {
  key: string;
  label: string;
  /** Where the entry leads: the module, or the first page of the view. */
  id: string;
  /** Every module id the entry stands for, so any of its pages marks it active. */
  ids: string[];
}

/** The usable modules as the sidebar lists them: one entry per view, in listing order. */
export function sidebarModules(modules: ModuleInfo[]): SidebarModule[] {
  const out: SidebarModule[] = [];
  const seen = new Set<string>();
  for (const m of modules) {
    if (m.error) continue;
    const paged = pagesOf(modules, m.id);
    if (!paged) {
      out.push({ key: m.id, label: m.name || m.id, id: m.id, ids: [m.id] });
      continue;
    }
    if (seen.has(paged.group.id)) continue;
    seen.add(paged.group.id);
    out.push({
      key: `connector:${paged.group.id}`,
      label: paged.group.name,
      id: paged.pages[0].id,
      ids: paged.pages.map((p) => p.id),
    });
  }
  return out;
}
