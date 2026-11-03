import type { ComponentPropsWithoutRef } from "react";

/**
 * A live number: the number face with tabular numerals (`.num`, DESIGN §18.1, §18.7 rule 5). Render readouts through
 * this or put `num` on the element; the layout audit (#457) checks the computed style of every live value.
 */
export function Num({ className, ...rest }: ComponentPropsWithoutRef<"span">) {
  return <span {...rest} className={className ? `${className} num` : "num"} />;
}
