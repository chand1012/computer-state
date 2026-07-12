import type { ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

export function Button({ className, variant = "default", ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "default" | "outline" | "ghost" | "danger" }) {
  return <button className={cn("button", `button-${variant}`, className)} {...props} />;
}
