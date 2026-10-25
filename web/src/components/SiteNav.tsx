import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import {
  ArrowUpRightIcon,
  DownloadSimpleIcon,
  GithubLogoIcon,
  ListIcon,
  XIcon,
} from "@phosphor-icons/react";
import { navLinks, site } from "../lib/site";

export default function SiteNav() {
  const [open, setOpen] = useState(false);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const reduceMotion = useReducedMotion();

  const handleAnchorClick = (
    event: React.MouseEvent<HTMLAnchorElement>,
    href: string,
  ) => {
    if (!href.startsWith("/#") || window.location.pathname !== "/") {
      setOpen(false);
      return;
    }

    event.preventDefault();
    setOpen(false);
    const id = href.slice(2);

    window.setTimeout(() => {
      const target = document.getElementById(id);
      if (!target) return;
      const top = target.getBoundingClientRect().top + window.scrollY - 112;
      window.scrollTo({
        top,
        behavior: reduceMotion ? "auto" : "smooth",
      });
      history.replaceState(null, "", href);
    }, 80);
  };

  useEffect(() => {
    if (!open) {
      return;
    }

    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    closeRef.current?.focus();

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      window.removeEventListener("keydown", onKeyDown);
      toggleRef.current?.focus();
    };
  }, [open]);

  return (
    <header className="fixed inset-x-0 top-4 z-50 px-4 sm:top-6">
      <nav
        aria-label="Primary"
        className="mx-auto flex w-full max-w-5xl items-center justify-between gap-3 rounded-full border border-ink/10 bg-paper/85 py-2 pl-4 pr-2 shadow-[0_12px_40px_rgba(18,17,14,0.08)] backdrop-blur-xl"
      >
        <a
          href="/"
          className="flex shrink-0 items-center gap-2.5"
          aria-label="Dunlin home"
        >
          <img
            src="/brand/dunlin-icon.svg"
            alt=""
            width="28"
            height="28"
            className="size-7 rounded-[8px]"
          />
          <span className="text-sm font-semibold tracking-tight text-ink">
            Dunlin
          </span>
        </a>

        <div className="hidden items-center gap-1 md:flex">
          {navLinks.map((link) => (
            <a
              key={link.href}
              href={link.href}
              onClick={(event) => handleAnchorClick(event, link.href)}
              className="rounded-full px-3 py-2 text-sm font-medium text-ink/70 transition-colors duration-700 ease-fluid hover:bg-ink/5 hover:text-ink"
            >
              {link.label}
            </a>
          ))}
          <a
            href={site.github}
            target="_blank"
            rel="noreferrer"
            className="ml-1 flex size-9 items-center justify-center rounded-full text-ink/70 transition-colors duration-700 ease-fluid hover:bg-ink/5 hover:text-ink"
            aria-label="Dunlin on GitHub"
          >
            <GithubLogoIcon size={18} weight="regular" />
          </a>
        </div>

        <div className="flex items-center gap-1.5">
          <a
            href="/download"
            className="hidden items-center gap-2 rounded-full bg-ink px-4 py-2.5 text-sm font-semibold text-paper-soft transition-transform duration-700 ease-fluid active:scale-[0.98] sm:inline-flex"
          >
            <DownloadSimpleIcon size={16} weight="bold" />
            Download
          </a>
          <button
            ref={toggleRef}
            type="button"
            onClick={() => setOpen(true)}
            className="flex size-10 items-center justify-center rounded-full text-ink transition-colors duration-700 ease-fluid hover:bg-ink/5 md:hidden"
            aria-expanded={open}
            aria-controls="mobile-menu"
            aria-label="Open menu"
          >
            <ListIcon size={22} weight="regular" />
          </button>
        </div>
      </nav>

      <AnimatePresence>
        {open && (
          <motion.div
            id="mobile-menu"
            role="dialog"
            aria-modal="true"
            aria-label="Menu"
            className="fixed inset-0 z-50 flex flex-col bg-paper px-6 pb-8 pt-5 md:hidden"
            initial={reduceMotion ? false : { opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={reduceMotion ? undefined : { opacity: 0 }}
            transition={{ duration: 0.45, ease: [0.32, 0.72, 0, 1] }}
          >
            <div className="flex items-center justify-between">
              <a
                href="/"
                className="flex items-center gap-2.5"
                onClick={() => setOpen(false)}
              >
                <img
                  src="/brand/dunlin-icon.svg"
                  alt=""
                  width="30"
                  height="30"
                  className="size-[30px] rounded-[8px]"
                />
                <span className="text-base font-semibold tracking-tight">
                  Dunlin
                </span>
              </a>
              <button
                ref={closeRef}
                type="button"
                onClick={() => setOpen(false)}
                className="flex size-11 items-center justify-center rounded-full border border-ink/10 text-ink"
                aria-label="Close menu"
              >
                <XIcon size={22} weight="regular" />
              </button>
            </div>

            <div className="mt-16 flex flex-col gap-2">
              {navLinks.map((link, index) => (
                <motion.a
                  key={link.href}
                  href={link.href}
                  onClick={(event) => handleAnchorClick(event, link.href)}
                  className="border-b border-ink/10 py-4 font-serif text-5xl leading-none tracking-tight text-ink"
                  initial={reduceMotion ? false : { opacity: 0, y: 24 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{
                    delay: reduceMotion ? 0 : 0.06 + index * 0.06,
                    duration: 0.6,
                    ease: [0.16, 1, 0.3, 1],
                  }}
                >
                  {link.label}
                </motion.a>
              ))}
            </div>

            <div className="mt-auto flex flex-col gap-3">
              <a
                href={site.github}
                target="_blank"
                rel="noreferrer"
                className="flex items-center justify-between rounded-full border border-ink/10 px-5 py-4 text-sm font-semibold text-ink"
              >
                View on GitHub
                <ArrowUpRightIcon size={18} />
              </a>
              <a
                href="/download"
                className="flex items-center justify-between rounded-full bg-ink px-5 py-4 text-sm font-semibold text-paper-soft"
                onClick={() => setOpen(false)}
              >
                Download the beta
                <DownloadSimpleIcon size={18} weight="bold" />
              </a>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </header>
  );
}
