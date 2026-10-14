// A place whose content lands with its own issue (#92 Today, #96 Workouts, #110 History, …). The shell owns
// the heading so the hand-off, skip link and focus order are real from day one.
import type { ReactNode } from "react";

export function Placeholder({
  title,
  children,
}: {
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="page">
      <div className="page-head">
        <h1>{title}</h1>
        <p className="muted">Coming soon.</p>
      </div>
      {children}
    </div>
  );
}
