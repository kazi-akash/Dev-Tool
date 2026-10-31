import {
  Activity,
  Cpu,
  HardDrive,
  History,
  LayoutDashboard,
  Settings,
  Stethoscope,
  Zap,
  type LucideIcon,
} from "lucide-react";

export interface NavItem {
  path: string;
  label: string;
  icon: LucideIcon;
}

export const NAV_ITEMS: NavItem[] = [
  { path: "/", label: "Dashboard", icon: LayoutDashboard },
  { path: "/processes", label: "Processes", icon: Cpu },
  { path: "/storage", label: "Storage", icon: HardDrive },
  { path: "/diagnosis", label: "Diagnosis", icon: Stethoscope },
  { path: "/actions", label: "Actions", icon: Zap },
  { path: "/history", label: "History", icon: History },
  { path: "/settings", label: "Settings", icon: Settings },
];

export const APP_ICON = Activity;
