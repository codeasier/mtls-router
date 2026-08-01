export type SectionId =
  "router" | "agents" | "api-keys" | "conversations" | "logs" | "settings";

export interface NavigationItem {
  id: SectionId;
  index: string;
}

export const navigationItems: NavigationItem[] = [
  { id: "router", index: "01" },
  { id: "agents", index: "02" },
  { id: "api-keys", index: "03" },
  { id: "conversations", index: "04" },
  { id: "logs", index: "05" },
  { id: "settings", index: "06" },
];
