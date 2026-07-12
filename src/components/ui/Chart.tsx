import type { ReactElement } from "react";
import { ResponsiveContainer } from "recharts";

// Local shadcn/ui-style chart primitive. Recharts owns rendering while this
// component owns responsive sizing and the application theme boundary.
export function ChartContainer({ children, className = "" }: { children: ReactElement; className?: string }) {
  return <div className={`chart-container ${className}`}><ResponsiveContainer width="100%" height="100%">{children}</ResponsiveContainer></div>;
}
