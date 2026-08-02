export type SectionId = "router" | "agents" | "api-keys" | "logs" | "settings";

export interface NavigationItem {
  id: SectionId;
  index: string;
}

export const navigationItems: NavigationItem[] = [
  { id: "router", index: "01" },
  { id: "agents", index: "02" },
  { id: "api-keys", index: "03" },
  { id: "logs", index: "04" },
  { id: "settings", index: "05" },
];
