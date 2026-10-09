import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Tide — run your house at low tide",
  description:
    "A composer for time-varying cost. Tide draws your electricity price curve, schedules your appliances into its cheapest windows, and proves the schedule is optimal.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <head>
        <link rel="icon" href="/favicon.svg" type="image/svg+xml" />
        <link rel="apple-touch-icon" href="/favicon.svg" />
      </head>
      <body>{children}</body>
    </html>
  );
}
