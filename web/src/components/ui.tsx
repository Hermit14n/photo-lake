import * as Popover from "@radix-ui/react-popover";
import * as RSelect from "@radix-ui/react-select";
import clsx from "clsx";
import { Check, ChevronDown, Loader2, Search, X } from "lucide-react";
import { forwardRef, useEffect, useMemo, useRef, useState, type ButtonHTMLAttributes, type ReactNode } from "react";

export function Button({
  variant = "default",
  size = "md",
  className,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: "default" | "primary" | "ghost" | "danger"; size?: "sm" | "md" }) {
  return (
    <button
      {...rest}
      className={clsx(
        "inline-flex items-center justify-center gap-1.5 rounded-lg font-medium transition-colors disabled:opacity-50 disabled:pointer-events-none whitespace-nowrap",
        size === "sm" ? "h-7 px-2.5 text-xs" : "h-9 px-3.5 text-sm",
        variant === "primary" && "bg-accent text-white hover:brightness-110",
        variant === "default" && "bg-s2 border border-line text-ink hover:bg-s3",
        variant === "ghost" && "text-ink2 hover:bg-s2 hover:text-ink",
        variant === "danger" && "bg-s2 border border-line text-danger hover:bg-s3",
        className,
      )}
    />
  );
}

export function Spinner({ className }: { className?: string }) {
  return <Loader2 className={clsx("animate-spin text-muted", className)} size={18} />;
}

export function Card({ children, className, title, action }: { children: ReactNode; className?: string; title?: ReactNode; action?: ReactNode }) {
  return (
    <section className={clsx("rounded-xl border border-line bg-s1", className)}>
      {(title || action) && (
        <header className="flex items-center justify-between px-4 pt-3.5 pb-1">
          <h3 className="text-sm font-semibold text-ink">{title}</h3>
          {action}
        </header>
      )}
      <div className="p-4 pt-2">{children}</div>
    </section>
  );
}

export const Input = forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(function Input(
  { className, ...rest },
  ref,
) {
  return (
    <input
      ref={ref}
      {...rest}
      className={clsx(
        "h-9 w-full rounded-lg border border-line bg-s2 px-3 text-base text-ink placeholder:text-muted outline-none focus:border-focus md:text-sm",
        className,
      )}
    />
  );
});

export function SearchInput({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder?: string }) {
  const [local, setLocal] = useState(value);
  useEffect(() => setLocal(value), [value]);
  useEffect(() => {
    if (local === value) return;
    const t = setTimeout(() => onChange(local), 300);
    return () => clearTimeout(t);
  }, [local]); // eslint-disable-line react-hooks/exhaustive-deps
  return (
    <div className="relative w-full md:w-auto">
      <Search size={15} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-muted" />
      <Input value={local} onChange={(e) => setLocal(e.target.value)} placeholder={placeholder} className="pl-8 md:w-56" />
    </div>
  );
}

export type Option = { value: string; label: string; count?: number };

const ANY = "__any__";

/** Dropdown select (Radix). An empty value means "Any". */
export function Select({
  value,
  onChange,
  options,
  placeholder,
  label,
  allowAny = true,
  className,
}: {
  value: string | undefined;
  onChange: (v: string | undefined) => void;
  options: Option[];
  placeholder: string;
  label?: string;
  allowAny?: boolean;
  className?: string;
}) {
  return (
    <RSelect.Root value={value || ANY} onValueChange={(v) => onChange(v === ANY ? undefined : v)}>
      <RSelect.Trigger
        aria-label={label ?? placeholder}
        className={clsx(
          "inline-flex h-9 min-w-28 items-center justify-between gap-2 rounded-lg border px-3 text-base outline-none md:text-sm",
          value ? "border-accent/60 bg-s2 text-ink" : "border-line bg-s2 text-ink2 hover:text-ink",
          className,
        )}
      >
        <span className="truncate max-w-44">
          {value ? options.find((o) => o.value === value)?.label ?? value : placeholder}
        </span>
        <ChevronDown size={14} className="shrink-0 text-muted" />
      </RSelect.Trigger>
      <RSelect.Portal>
        <RSelect.Content position="popper" sideOffset={4} className="z-50 max-h-80 min-w-48 max-w-[calc(100vw-16px)] overflow-hidden rounded-lg border border-line bg-s1 shadow-xl">
          <RSelect.Viewport className="p-1">
            {allowAny && <Item value={ANY} label={`Any ${placeholder.toLowerCase()}`} />}
            {options.map((o) => (
              <Item key={o.value} value={o.value} label={o.label} count={o.count} />
            ))}
          </RSelect.Viewport>
        </RSelect.Content>
      </RSelect.Portal>
    </RSelect.Root>
  );
}

function Item({ value, label, count }: { value: string; label: string; count?: number }) {
  return (
    <RSelect.Item
      value={value}
      className="relative flex cursor-pointer select-none items-center justify-between gap-4 rounded-md py-1.5 pl-7 pr-2 text-sm text-ink outline-none data-[highlighted]:bg-s3"
    >
      <RSelect.ItemIndicator className="absolute left-2">
        <Check size={13} />
      </RSelect.ItemIndicator>
      <RSelect.ItemText>{label}</RSelect.ItemText>
      {count !== undefined && <span className="text-xs tabular-nums text-muted">{count}</span>}
    </RSelect.Item>
  );
}

/** Searchable dropdown for long lists (species). `onSearch` enables server-side lookup. */
export function Combobox({
  value,
  valueLabel,
  onChange,
  options,
  placeholder,
  onSearch,
  trigger,
  emptyText = "No matches",
}: {
  value?: string;
  valueLabel?: string;
  onChange: (v: string | undefined, o?: Option) => void;
  options: Option[];
  placeholder: string;
  onSearch?: (q: string) => void;
  trigger?: ReactNode;
  emptyText?: string;
}) {
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const shown = useMemo(() => {
    if (onSearch) return options;
    const ql = q.toLowerCase();
    return ql ? options.filter((o) => o.label.toLowerCase().includes(ql)) : options;
  }, [options, q, onSearch]);
  useEffect(() => {
    onSearch?.(q);
    setActive(0);
  }, [q]); // eslint-disable-line react-hooks/exhaustive-deps
  const pick = (o?: Option) => {
    onChange(o?.value, o);
    setOpen(false);
    setQ("");
  };
  const label = value ? valueLabel ?? options.find((o) => o.value === value)?.label ?? value : placeholder;
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        {trigger ?? (
          <button
            className={clsx(
              "inline-flex h-9 min-w-36 items-center justify-between gap-2 rounded-lg border px-3 text-base md:text-sm",
              value ? "border-accent/60 bg-s2 text-ink" : "border-line bg-s2 text-ink2 hover:text-ink",
            )}
          >
            <span className="truncate max-w-48">{label}</span>
            <ChevronDown size={14} className="shrink-0 text-muted" />
          </button>
        )}
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content sideOffset={4} align="start" className="z-50 w-80 max-w-[calc(100vw-16px)] rounded-lg border border-line bg-s1 p-1 shadow-xl">
          <Input
            autoFocus
            value={q}
            placeholder={`Search ${placeholder.toLowerCase()}…`}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") setActive((a) => Math.min(a + 1, shown.length - 1));
              else if (e.key === "ArrowUp") setActive((a) => Math.max(a - 1, 0));
              else if (e.key === "Enter" && shown[active]) pick(shown[active]);
              else return;
              e.preventDefault();
            }}
          />
          <div ref={listRef} className="mt-1 max-h-72 overflow-auto">
            {value && (
              <button onClick={() => pick(undefined)} className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-ink2 hover:bg-s3">
                <X size={13} /> Clear
              </button>
            )}
            {shown.length === 0 && <div className="px-2 py-3 text-center text-sm text-muted">{emptyText}</div>}
            {shown.map((o, i) => (
              <button
                key={o.value}
                onMouseEnter={() => setActive(i)}
                onClick={() => pick(o)}
                className={clsx(
                  "flex w-full items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-sm text-ink",
                  i === active && "bg-s3",
                )}
              >
                <span className="truncate">{o.label}</span>
                {o.count !== undefined && <span className="text-xs tabular-nums text-muted">{o.count}</span>}
              </button>
            ))}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

export function Chip({ children, onRemove }: { children: ReactNode; onRemove?: () => void }) {
  return (
    <span className="inline-flex h-7 items-center gap-1 rounded-full border border-line bg-s2 pl-2.5 pr-1.5 text-xs text-ink">
      {children}
      {onRemove && (
        <button onClick={onRemove} aria-label="Remove filter" className="rounded-full p-0.5 text-muted hover:bg-s3 hover:text-ink">
          <X size={12} />
        </button>
      )}
    </span>
  );
}

export function Segmented<T extends string>({ value, onChange, options }: { value: T; onChange: (v: T) => void; options: { value: T; label: ReactNode }[] }) {
  return (
    <div className="inline-flex h-9 rounded-lg border border-line bg-s2 p-0.5" role="radiogroup">
      {options.map((o) => (
        <button
          key={o.value}
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={clsx(
            "inline-flex items-center gap-1.5 rounded-md px-3 text-sm",
            value === o.value ? "bg-s3 text-ink shadow-sm" : "text-ink2 hover:text-ink",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Empty({ icon, title, children }: { icon?: ReactNode; title: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-24 text-center">
      {icon && <div className="text-muted">{icon}</div>}
      <div className="text-base font-medium text-ink">{title}</div>
      {children && <div className="max-w-md text-sm text-ink2">{children}</div>}
    </div>
  );
}
