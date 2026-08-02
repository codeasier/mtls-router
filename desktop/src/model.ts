export type SectionId = "router" | "agents" | "api-keys" | "logs" | "settings";

export interface NavigationItem {
  id: SectionId;
}

export const navigationItems: NavigationItem[] = [
  { id: "router" },
  { id: "agents" },
  { id: "api-keys" },
  { id: "logs" },
  { id: "settings" },
];
