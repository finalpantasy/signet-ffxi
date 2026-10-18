---
name: Signet for FFXI
description: Compact profile, addon, and graphics controls in a frosted-white desktop utility.
colors:
  accent: "#007c91"
  focus: "#006a7c"
  canvas: "#e9efef"
  surface: "#f5f9f8"
  glass: "rgba(249, 252, 251, 0.84)"
  text: "#243436"
  secondary: "#53676a"
  line: "#cedcdd"
  white: "#ffffff"
typography:
  title:
    fontFamily: "Segoe UI, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.4
  body:
    fontFamily: "Segoe UI, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: "Segoe UI, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  xs: "4px"
  control: "6px"
  shell: "10px"
  toggle: "12px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  2xl: "24px"
components:
  primary-action:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.white}"
    rounded: "{rounded.control}"
    padding: "6px 12px"
    height: "34px"
  secondary-action:
    backgroundColor: "#e0ebeb"
    textColor: "#25484e"
    rounded: "{rounded.control}"
    padding: "6px 12px"
    height: "34px"
  input:
    backgroundColor: "{colors.white}"
    textColor: "{colors.text}"
    rounded: "{rounded.control}"
    padding: "6px 9px"
    height: "34px"
  navigation-item:
    textColor: "{colors.secondary}"
    rounded: "{rounded.xs}"
    padding: "8px 12px"
    height: "40px"
  toggle:
    backgroundColor: "#e0e8e8"
    rounded: "{rounded.toggle}"
    width: "34px"
    height: "20px"
  dialog:
    backgroundColor: "#f7fbfa"
    textColor: "{colors.text}"
    rounded: "{rounded.shell}"
    padding: "20px"
    width: "min(460px, calc(100% - 32px))"
---

# Design System: Signet for FFXI

## Overview

**Creative North Star: "Ultralight Utility"**

The player interface is a compact Windows utility with a pale, frosted surface, direct labels, and legible sans-serif type. A single teal accent makes the primary action easy to find. The browser preview uses CSS blur to suggest frosted glass; this treatment describes the preview, not a native Windows backdrop.

Keep the working area task-first: choose a profile, move between a few clear sections, and edit settings in place. Copy stays brief, technical options sit behind Advanced disclosures, and the Play flow states the preview limitation clearly. The build has no scenic artwork, hero headings, slogans, or decorative dashboard blocks.

**Key Characteristics:**

- Compact utility framing with calm pale teal-gray surfaces.
- Segoe UI labels and readable control text.
- One clear teal action, with restrained selected and focus states.
- Simple rows and forms; advanced controls stay tucked away.

**The Utility Frame Rule.** Keep screens compact and task-first, with direct labels and no oversized introduction.

## Colors

The palette is neutral-first: pale teal-gray surfaces carry most of the screen, dark ink maintains legibility, and teal marks important actions and states.

### Primary

- **Utility Teal:** The main Play action, enabled switches, and active navigation underline use the single saturated accent.
- **Deep Focus Teal:** Keyboard focus and selected text use a darker teal for clear state contrast.

### Neutral

- **Cool Canvas:** The page backdrop frames the app surface.
- **Frosted Surface:** The translucent app shell sits over the canvas; an opaque preference uses the solid surface.
- **Ink and Muted Ink:** Primary labels use dark ink; secondary labels and helper copy use muted ink.
- **Quiet Divider:** Thin separators organize list rows and sections.
- **Control White:** Inputs and fields use a clean white fill.

**The Single Accent Rule.** Let teal identify the primary action and active state; keep surrounding surfaces and secondary actions quiet.

## Typography

**Display Font:** None; this system has no display face.
**Body Font:** Single UI sans-serif family
**Label/Mono Font:** Same family as body

**Character:** A single familiar sans-serif family keeps navigation, settings, and sample data legible at compact sizes. Text hierarchy comes from modest size and weight changes rather than decorative display type.

### Hierarchy

- **Title:** Compact page and section titles; dialog headings step up slightly.
- **Body:** Controls, profile details, and general reading copy.
- **Label:** Quiet status and helper text; the interface text-size preference scales the root size.

**The Legible Scale Rule.** Keep one sans-serif voice and a restrained size ladder; let labels remain concise.

## Layout

Center the working surface in a narrow desktop frame, with the profile picker and Play action above four simple text tabs. Use a modest content inset and open, divided rows instead of a card grid. At the mobile breakpoint (600px), the frame narrows to a 12px outer gutter, profile choices become a two-column list, paired fields stack, and library rows wrap their actions.

**The Open List Rule.** Prefer aligned controls and thin row dividers; group content only when the task needs it.

## Elevation & Depth

Depth comes from one soft shadow and CSS backdrop blur on the outer preview surface, plus a stronger shadow and dimmed backdrop for dialogs. Dividers and pale tonal changes do most of the grouping work. Reduced-transparency settings remove blur and use a solid surface.

**The Preview Glass Rule.** Apply translucency only to the preview shell; always provide an opaque surface state.

## Shapes

Controls and selected rows share a gently rounded rectangle. The outer shell and dialog use a broader corner, while switches use a capsule. Inputs retain a fine border, and list hierarchy comes from separators rather than nested outlined cards.

**The Shared Control Rule.** Keep buttons, fields, and profile choices within one control shape family; reserve the capsule silhouette for switches.

## Components

- **Primary action:** Teal fill, white text, compact height, and a visible keyboard focus ring. Hover darkens the fill.
- **Secondary action:** Pale teal fill with dark teal text; use it for adjacent, lower-emphasis actions.
- **Inputs:** White fill, fine border, clear labels, and a teal focus ring. Range controls pair with an editable numeric value where needed.
- **Navigation:** Text-only tabs use muted labels at rest and a teal underline for the active page.
- **Selection rows:** Profile choices use a pale selected fill. Summary and library rows stay open with thin dividers.
- **Switches:** A small labeled switch shows enabled state with the teal accent and a contrasting thumb.
- **Dialogs:** Keep the title, close control, concise content, and actions in one compact surface.
- **Advanced options:** Use a native disclosure summary for less common graphics and app settings.

**The Semantic Control Rule.** Use labeled controls and visible focus; encode state with a clear text, fill, or underline change.

## Do's and Don'ts

### Do:

- Do keep profile choice and the primary Play action near the top.
- Do use short labels, aligned controls, and open rows for everyday tasks.
- Do preserve visible focus, text scaling, and the opaque appearance option.
- Do place less common power options behind Advanced disclosures.
- Do state preview scope clearly in the Play flow.

### Don't:

- Don't add scenic artwork, large hero headings, slogans, or decorative dashboard panels.
- Don't repeat explanatory subtitles where a concise label or one Play-flow note is enough.
- Don't imply that the browser preview launches FFXI, installs mods, or applies settings.

**Not canonized:** Decorative illustration and display typography are absent from the built interface, so they do not define reusable parts of this system.
