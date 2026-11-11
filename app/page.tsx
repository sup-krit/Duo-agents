import { listSessions } from "@/lib/session";
import { StartForm } from "@/components/duo/start-form";

export const dynamic = "force-dynamic";

export default function Home() {
  return <StartForm sessions={listSessions()} />;
}
