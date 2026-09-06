import type { SectionId } from "./model";

function IconShell({ children }: { children: React.ReactNode }) {
  return (
    <svg
      aria-hidden="true"
      className="nav-marker__icon"
      viewBox="0 0 16 16"
      focusable="false"
    >
      {children}
    </svg>
  );
}

export function NavIcon({ id }: { id: SectionId }) {
  switch (id) {
    case "router":
      return (
        <IconShell>
          <circle cx="3.3" cy="8" r="1.4" />
          <circle cx="12.5" cy="4.1" r="1.4" />
          <circle cx="12.5" cy="11.9" r="1.4" />
          <path d="M4.7 8H7.1L11.4 4.6M7.1 8l4.3 3.4" />
        </IconShell>
      );
    case "agents":
      return (
        <IconShell>
          <rect x="2.7" y="3.1" width="10.6" height="9.8" rx="2" />
          <path d="M5.4 6.5 7.3 8l-1.9 1.5M8.3 10.3h2.5" />
        </IconShell>
      );
    case "api-keys":
      return (
        <IconShell>
          <circle cx="5.3" cy="8" r="2.45" />
          <path d="M7.6 8h6.1v1.7h-1.5v1.5h-1.7V9.7H10" />
        </IconShell>
      );
    case "usage":
      return (
        <IconShell>
          <path d="M3.2 12.2V8.4h2.3V12.2zM6.85 12.2V4.6h2.3v7.6zM10.5 12.2V6.7h2.3v5.5z" />
        </IconShell>
      );
    case "logs":
      return (
        <IconShell>
          <path d="M4.2 2.8h5.3L12 5.4v7.8H4.2z" />
          <path d="M9.5 2.8v2.7H12" />
          <path d="M6.1 8.1h3.9M6.1 10.4h2.7" />
        </IconShell>
      );
    case "settings":
      return (
        <IconShell>
          <path d="M8 2.7 9.55 3.6l1.7-.25.85 1.5 1.5.85-.25 1.7L14.25 8l-.9 1.55.25 1.7-1.5.85-.85 1.5-1.7-.25L8 13.3l-1.55.9-1.7.25-.85-1.5-1.5-.85.25-1.7L1.75 8l.9-1.55-.25-1.7 1.5-.85.85-1.5 1.7.25z" />
          <circle cx="8" cy="8" r="2.05" />
        </IconShell>
      );
  }
}
