import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { HashRouter, Route, Routes } from "react-router-dom";
import { Toaster } from "sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AppShell } from "@/components/layout/AppShell";
import { EmergencyStopProgressDialog } from "@/components/emergency/EmergencyStop";
import { useBandenEvents } from "@/hooks/useBanden";
import Dashboard from "@/pages/Dashboard";
import Devices from "@/pages/Devices";
import NetworkMap from "@/pages/NetworkMap";
import Traffic from "@/pages/Traffic";
import Controls from "@/pages/Controls";
import Activity from "@/pages/Activity";
import Settings from "@/pages/Settings";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      refetchOnWindowFocus: false,
      staleTime: 2000,
    },
  },
});

function BandenApp() {
  useBandenEvents();
  return (
    <>
      <HashRouter>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/" element={<Dashboard />} />
            <Route path="/devices" element={<Devices />} />
            <Route path="/map" element={<NetworkMap />} />
            <Route path="/traffic" element={<Traffic />} />
            <Route path="/controls" element={<Controls />} />
            <Route path="/activity" element={<Activity />} />
            <Route path="/settings" element={<Settings />} />
            <Route path="*" element={<Dashboard />} />
          </Route>
        </Routes>
      </HashRouter>
      <EmergencyStopProgressDialog />
      <Toaster position="bottom-right" richColors closeButton />
    </>
  );
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={200}>
        <BandenApp />
      </TooltipProvider>
    </QueryClientProvider>
  );
}
