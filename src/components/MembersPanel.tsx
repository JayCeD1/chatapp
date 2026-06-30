import React from "react";
import { initials, avatarColor } from "../utils";

export interface Member {
  name: string;
  online: boolean;
  isYou: boolean;
}

interface MembersPanelProps {
  members: Member[];
}

export const MembersPanel: React.FC<MembersPanelProps> = ({ members }) => {
  const online = members.filter((m) => m.online);
  const offline = members.filter((m) => !m.online);

  const Row = ({ m }: { m: Member }) => (
    <li className="flex items-center gap-2.5 px-3 py-1.5 rounded-md hover:bg-[var(--surface-2)] transition-colors">
      <div className="relative shrink-0">
        <div
          className={`flex items-center justify-center w-7 h-7 rounded-full text-[11px] font-semibold text-white ${
            m.online ? "" : "opacity-50"
          }`}
          style={{ background: avatarColor(m.name) }}
        >
          {initials(m.name)}
        </div>
        <span
          className="absolute -bottom-0.5 -right-0.5 w-2.5 h-2.5 rounded-full border-2 border-[var(--surface)]"
          style={{ background: m.online ? "var(--online)" : "#57606a" }}
        />
      </div>
      <span
        className={`text-sm truncate ${
          m.online ? "text-[var(--text)]" : "text-[var(--text-faint)]"
        }`}
      >
        {m.name}
        {m.isYou && <span className="text-[var(--text-faint)]"> (you)</span>}
      </span>
    </li>
  );

  return (
    <aside className="hidden lg:flex h-full flex-col bg-[var(--surface)] border-l border-[var(--border)]">
      <div className="flex items-center px-4 h-14 border-b border-[var(--border)] shrink-0">
        <span className="text-sm font-semibold text-[var(--text)]">People</span>
        <span className="ml-2 text-[11px] text-[var(--text-faint)]">
          {members.length}
        </span>
      </div>
      <div className="flex-1 min-h-0 overflow-y-auto py-2 scrollbar-thin scrollbar-thumb-white/10 scrollbar-track-transparent">
        {online.length > 0 && (
          <Section title={`Online — ${online.length}`}>
            {online.map((m) => (
              <Row key={m.name} m={m} />
            ))}
          </Section>
        )}
        {offline.length > 0 && (
          <Section title={`Offline — ${offline.length}`}>
            {offline.map((m) => (
              <Row key={m.name} m={m} />
            ))}
          </Section>
        )}
        {members.length === 0 && (
          <p className="px-4 py-3 text-sm text-[var(--text-faint)]">
            No one here yet.
          </p>
        )}
      </div>
    </aside>
  );
};

const Section: React.FC<{ title: string; children: React.ReactNode }> = ({
  title,
  children,
}) => (
  <div className="mb-3">
    <div className="px-4 mb-1 text-[11px] font-semibold uppercase tracking-wider text-[var(--text-faint)]">
      {title}
    </div>
    <ul className="px-2 space-y-0.5">{children}</ul>
  </div>
);
