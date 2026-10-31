import { NavLink } from "react-router-dom";
import { APP_ICON, NAV_ITEMS } from "@/app/navigation";
import { cn } from "@/lib/utils";

export function Sidebar() {
  const Logo = APP_ICON;
  return (
    <aside className="flex w-52 shrink-0 flex-col border-r bg-card">
      <div className="flex h-14 items-center gap-2 px-4">
        <Logo className="size-5 text-primary" />
        <span className="text-sm font-semibold tracking-tight">DevPilot</span>
      </div>
      <nav className="flex flex-1 flex-col gap-0.5 p-2" aria-label="Main">
        {NAV_ITEMS.map(({ path, label, icon: Icon }) => (
          <NavLink
            key={path}
            to={path}
            end={path === "/"}
            className={({ isActive }) =>
              cn(
                "flex items-center gap-2.5 rounded-md px-3 py-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground",
                isActive && "bg-accent font-medium text-accent-foreground",
              )
            }
          >
            <Icon className="size-4" />
            {label}
          </NavLink>
        ))}
      </nav>
      <div className="border-t px-4 py-3 text-xs text-muted-foreground">v0.1.0</div>
    </aside>
  );
}
