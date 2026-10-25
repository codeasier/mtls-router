import { useEffect, useRef } from "react";

import { ChangelogMarkdown } from "./changelogMarkdown";
import type { ChangelogEntry } from "./releaseNotes";

interface ReleaseHistoryDialogProps {
  title: string;
  closeLabel: string;
  emptyLabel: string;
  entries: ChangelogEntry[];
  selectedVersion: string | null;
  onSelect(version: string): void;
  onClose(): void;
}

export function ReleaseHistoryDialog({
  title,
  closeLabel,
  emptyLabel,
  entries,
  selectedVersion,
  onSelect,
  onClose,
}: ReleaseHistoryDialogProps) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const selected =
    entries.find((entry) => entry.version === selectedVersion) ??
    entries[0] ??
    null;

  useEffect(() => {
    closeRef.current?.focus();
  }, []);

  useEffect(() => {
    function escape(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [onClose]);

  return (
    <div
      className="dialog-backdrop"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <section
        className="history-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="history-dialog-title"
      >
        <div className="history-dialog__head">
          <h2 id="history-dialog-title">{title}</h2>
          <button
            ref={closeRef}
            type="button"
            className="text-button"
            onClick={onClose}
          >
            {closeLabel}
          </button>
        </div>
        {selected ? (
          <div className="changelog-browser">
            <div
              className="changelog-browser__versions"
              role="group"
              aria-label={title}
            >
              {entries.map((entry) => (
                <button
                  key={entry.version}
                  type="button"
                  aria-current={
                    entry.version === selected.version ? "true" : undefined
                  }
                  onClick={() => onSelect(entry.version)}
                >
                  <strong>{`v${entry.version}`}</strong>
                  {entry.date ? <span>{entry.date}</span> : null}
                </button>
              ))}
            </div>
            <div
              className="changelog-browser__body"
              tabIndex={0}
              aria-label={`v${selected.version}`}
            >
              <ChangelogMarkdown source={selected.body} />
            </div>
          </div>
        ) : (
          <p className="settings-block__status">{emptyLabel}</p>
        )}
      </section>
    </div>
  );
}
