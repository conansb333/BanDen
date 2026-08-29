/** Real brand icons for the app catalog (SVGs bundled from Simple Icons). */
import whatsapp from "@/assets/apps/whatsapp.svg";
import facebook from "@/assets/apps/facebook.svg";
import instagram from "@/assets/apps/instagram.svg";
import youtube from "@/assets/apps/youtube.svg";
import tiktok from "@/assets/apps/tiktok.svg";
import netflix from "@/assets/apps/netflix.svg";
import spotify from "@/assets/apps/spotify.svg";
import snapchat from "@/assets/apps/snapchat.svg";
import telegram from "@/assets/apps/telegram.svg";
import x from "@/assets/apps/x.svg";
import discord from "@/assets/apps/discord.svg";
import roblox from "@/assets/apps/roblox.svg";

const ICONS: Record<string, string> = {
  whatsapp,
  facebook,
  instagram,
  youtube,
  tiktok,
  netflix,
  spotify,
  snapchat,
  telegram,
  x,
  discord,
  roblox,
};

export function AppIcon({ id, name }: { id: string; name: string }) {
  const src = ICONS[id];
  if (!src) return null;
  return <img src={src} alt="" width={16} height={16} className="inline-block shrink-0" title={name} />;
}
