import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md" | "lg";

const base =
  "inline-flex items-center justify-center gap-2 rounded-md font-medium whitespace-nowrap select-none " +
  "transition-[background-color,border-color,color,transform] duration-(--duration-fast) ease-(--ease-out) " +
  "active:scale-[0.98] disabled:pointer-events-none disabled:opacity-45";

const variants: Record<Variant, string> = {
  primary: "bg-accent text-accent-fg hover:bg-accent-hover active:bg-accent-press",
  secondary: "bg-surface-3 text-fg border border-line hover:border-line-strong hover:bg-surface-3/80",
  ghost: "text-fg-muted hover:text-fg hover:bg-surface-3",
  danger: "bg-danger-soft text-danger border border-danger/30 hover:bg-danger/20",
};

const sizes: Record<Size, string> = {
  sm: "h-7 px-2.5 text-xs",
  md: "h-9 px-3.5 text-sm",
  lg: "h-11 px-5 text-[15px]",
};

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: Variant;
  size?: Size;
  icon?: ReactNode;
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon, className = "", children, type = "button", ...rest },
  ref,
) {
  return (
    <button ref={ref} type={type} className={`${base} ${variants[variant]} ${sizes[size]} ${className}`} {...rest}>
      {icon}
      {children}
    </button>
  );
});
