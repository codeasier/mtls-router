export type SectionId = "router" | "agents" | "logs" | "settings";

export interface NavigationItem {
  id: SectionId;
  index: string;
}

export const navigationItems: NavigationItem[] = [
  { id: "router", index: "01" },
  { id: "agents", index: "02" },
  { id: "logs", index: "03" },
  { id: "settings", index: "04" },
];
