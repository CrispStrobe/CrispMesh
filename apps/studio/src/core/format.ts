/**
 * Small display formatters. Fixed layouts (so screenshots and tests are stable); the only
 * language-dependent part is the decimal sign, "," in German (i18n `localDecimal`).
 */

import { language, localDecimal } from "../i18n/core";

export function formatDuration(seconds: number | undefined): string {
  if (seconds === undefined || !Number.isFinite(seconds)) return "";
  const total = Math.max(0, seconds);
  if (total < 60) return `${total < 10 ? localDecimal(total.toFixed(1)) : Math.round(total)} s`;
  const whole = Math.round(total);
  const minutes = Math.floor(whole / 60);
  const rest = whole % 60;
  if (minutes < 60) return `${minutes} min ${String(rest).padStart(2, "0")} s`;
  return `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, "0")} min`;
}

export function formatBytes(bytes: number | undefined): string {
  if (bytes === undefined || !Number.isFinite(bytes)) return "";
  return localDecimal(bytesText(bytes));
}

function bytesText(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  if (bytes < 1e6) return `${(bytes / 1e3).toFixed(bytes < 1e4 ? 1 : 0)} kB`;
  if (bytes < 1e9) return `${(bytes / 1e6).toFixed(bytes < 1e7 ? 1 : 0)} MB`;
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

/** 1234567 -> "1 234 567" (thin, locale-free grouping). */
export function formatCount(value: number | undefined): string {
  if (value === undefined || !Number.isFinite(value)) return "";
  return Math.round(value)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, " ");
}

export function formatNumber(value: number): string {
  return localDecimal(numberText(value));
}

function numberText(value: number): string {
  if (!Number.isFinite(value)) return String(value);
  if (Number.isInteger(value)) return formatCount(value);
  const magnitude = Math.abs(value);
  if (magnitude >= 100) return value.toFixed(1);
  if (magnitude >= 1) return value.toFixed(3);
  if (magnitude >= 0.001) return value.toFixed(4);
  return value.toExponential(2);
}

export function formatPercent(fraction: number, digits = 1): string {
  const number = localDecimal((fraction * 100).toFixed(digits));
  // German sets a (narrow, non-breaking) space before the percent sign.
  return language() === "de" ? `${number}\u202f%` : `${number}%`;
}

/** `coverage_level_1` -> "Coverage level 1". */
export function humanise(name: string): string {
  const spaced = name.replace(/[_-]+/g, " ").trim();
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

export function formatClock(unixSeconds: number | null | undefined): string {
  if (unixSeconds === null || unixSeconds === undefined || !Number.isFinite(unixSeconds)) return "";
  const date = new Date(unixSeconds * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
