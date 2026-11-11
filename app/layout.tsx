import type { Metadata, Viewport } from "next";
import "./globals.css";

export const metadata: Metadata = { title: "Agent Duo", description: "One agent leads, the team builds, sub-agents split the work." };
export const viewport: Viewport = { themeColor: "#0c0d0f", width: "device-width", initialScale: 1 };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500;700&display=swap" />
      </head>
      <body>{children}</body>
    </html>
  );
}
