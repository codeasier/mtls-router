import { Fragment, type ReactNode } from "react";

function inlineNodes(source: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let rest = source;
  let key = 0;
  while (rest.length > 0) {
    const code = /^`([^`]+)`/.exec(rest);
    const strong = /^\*\*([^*]+)\*\*/.exec(rest);
    const link = /^\[([^\]]+)\]\(([^()\s]+)\)/.exec(rest);
    if (code?.[1]) {
      nodes.push(<code key={key}>{code[1]}</code>);
      key += 1;
      rest = rest.slice(code[0].length);
      continue;
    }
    if (strong?.[1]) {
      nodes.push(<strong key={key}>{inlineNodes(strong[1])}</strong>);
      key += 1;
      rest = rest.slice(strong[0].length);
      continue;
    }
    if (link?.[1] && link[2]) {
      const label = inlineNodes(link[1]);
      if (/^https:\/\//i.test(link[2])) {
        nodes.push(
          <a key={key} href={link[2]} target="_blank" rel="noopener noreferrer">
            {label}
          </a>,
        );
      } else {
        nodes.push(<Fragment key={key}>{label}</Fragment>);
      }
      key += 1;
      rest = rest.slice(link[0].length);
      continue;
    }
    const next = rest.search(/`|\*\*|\[/);
    if (next < 0) {
      nodes.push(rest);
      break;
    }
    if (next === 0) {
      nodes.push(rest[0]);
      rest = rest.slice(1);
      continue;
    }
    nodes.push(rest.slice(0, next));
    rest = rest.slice(next);
  }
  return nodes;
}

/** Renders the changelog subset: headings, lists, bold, code, and https links. */
export function ChangelogMarkdown({ source }: { source: string }) {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const blocks: ReactNode[] = [];
  let index = 0;
  let key = 0;
  while (index < lines.length) {
    const line = lines[index] ?? "";
    if (!line.trim()) {
      index += 1;
      continue;
    }
    const heading = /^(#{1,3})\s+(.+)$/.exec(line);
    if (heading?.[1] && heading[2]) {
      const content = inlineNodes(heading[2]);
      blocks.push(
        heading[1].length <= 2 ? (
          <h3 key={key}>{content}</h3>
        ) : (
          <h4 key={key}>{content}</h4>
        ),
      );
      key += 1;
      index += 1;
      continue;
    }
    if (/^[-*]\s+/.test(line)) {
      const items: string[] = [];
      while (index < lines.length && /^[-*]\s+/.test(lines[index] ?? "")) {
        items.push((lines[index] ?? "").replace(/^[-*]\s+/, ""));
        index += 1;
      }
      blocks.push(
        <ul key={key}>
          {items.map((item, itemIndex) => (
            <li key={itemIndex}>{inlineNodes(item)}</li>
          ))}
        </ul>,
      );
      key += 1;
      continue;
    }
    const paragraph: string[] = [];
    while (
      index < lines.length &&
      (lines[index] ?? "").trim() &&
      !/^(#{1,3})\s+/.test(lines[index] ?? "") &&
      !/^[-*]\s+/.test(lines[index] ?? "")
    ) {
      paragraph.push((lines[index] ?? "").trim());
      index += 1;
    }
    blocks.push(<p key={key}>{inlineNodes(paragraph.join(" "))}</p>);
    key += 1;
  }
  return <div className="changelog-markdown">{blocks}</div>;
}
