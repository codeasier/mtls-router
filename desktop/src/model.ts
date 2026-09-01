export type SectionId =
  "router" | "agents" | "api-keys" | "usage" | "logs" | "settings";

export interface NavigationItem {
  id: SectionId;
}

export const navigationItems: NavigationItem[] = [
  { id: "router" },
  { id: "agents" },
  { id: "api-keys" },
  { id: "usage" },
  { id: "logs" },
  { id: "settings" },
];
