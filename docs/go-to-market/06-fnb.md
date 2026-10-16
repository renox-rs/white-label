# 06 — Cloud Kitchens & Local F&B Chains: Go-to-Market Pack

> Status: draft · Last updated: 2026-10-15 · Source:
> [06-fnb.md](../verticals/06-fnb.md) (accepted)

## 1. Positioning

**One line:** Every aggregator and your own ordering site on one kitchen screen, with daily
food cost and payout reconciliation, for UAE cloud kitchens and delivery-first chains.

**For:** cloud-kitchen operators running several virtual brands, and local F&B chains whose
business is mostly delivery, pickup and counter takeaway, often with a central kitchen.

**Not for (say so early):**
- Dine-in-led restaurants that need tables, coursing, split bills, service charge or table
  reservations. Dine-in is on the roadmap, not in the product today.
- Chains that want to keep their current POS as the source of sales: importing sales from
  another POS (e.g. Foodics) is on the roadmap, so food cost from their sales is not possible
  yet.
- Operators who want direct aggregator API links without middleware. Aggregator orders come
  in by quick entry on a tablet or through the client's own Deliverect subscription.
- Single cafés or one-brand kitchens with low volume, who are better served by a basic POS.

## 2. Ideal client profile

| | Fits | Weak fit |
|---|---|---|
| Operation | 1–5 kitchens with 2–15 virtual brands, or a chain of 3–30 outlets with a central kitchen | one outlet, one brand |
| Volume | 200–5,000 orders a day, most from aggregators | a few dozen orders a day |
| Team | 20–400 staff: kitchen managers, chefs, storekeeper, menu manager, accountant | owner does everything |
| Location | Dubai, Abu Dhabi, Sharjah | — |
| Channels | several aggregators plus phone and walk-in; wants its own direct ordering | dine-in is most of revenue |
| Today | one tablet per aggregator, recipes and food cost in Excel, payouts checked by hand or not at all, paper temperature logs | already on a full restaurant suite and happy with it |

**Signals that a company fits**
- Several brands on the aggregator apps cooking from the same address or area.
- Job ads for a "cost controller", "storekeeper", "food safety / PIC" or "aggregator
  operations" role (manual work we remove).
- A new kitchen or a new brand launched; a central kitchen opened.
- Owners talking publicly about commissions or wanting more direct orders.

**Who decides and who uses it**

| Role | Cares about | Message angle |
|---|---|---|
| Owner / operations director | Margin per brand and channel, commissions, growth | Net revenue per channel after commission, and where food cost leaks |
| Area manager | Several kitchens running the same way | Closings, food cost and checklists for every outlet on one dashboard |
| Kitchen manager | Rush hour, late orders, stock, inspections | One screen instead of four tablets; temperature logs and checklists ready for inspection |
| Head chef | Recipes, prep, waste | Recipes with costs, batch labels with expiry and allergens, wastage with reasons |
| Accountant | Aggregator payouts, supplier bills, VAT | Payout statements matched to orders; differences listed |

## 3. Lead sources

Build the list by hand: look up each company, check its website, brands and LinkedIn, and
record where it came from. Do not scrape or bulk-copy any of the sources below.

| Source | What it gives | Type | How to use | Terms |
|---|---|---|---|---|
| Dubai Municipality FoodWatch and the Dubai Municipality app (food safety ratings) [S1], [S2] | Licensed food establishments in Dubai and their food safety rating | Official | Look up a known outlet to confirm it is licensed and active; a weak rating hints at pain with records | Terms not checked; one company at a time |
| National Economic Register, via u.ae ([NER](https://u.ae/en/information-and-services/business/important-digital-services/national-economic-register)) [S3] | Licences and activities in all emirates | Official | Confirm a company's licence and activities (restaurant, food delivery, central kitchen) | Terms not checked |
| Dubai Chambers Restaurants Business Group [S4] | Restaurant operators in Dubai working on sector issues | Semi-official | Join or attend Chamber events; current activity of the group not confirmed | Membership |
| Aggregator apps (Talabat, Deliveroo, Careem, Noon Food, Keeta) | Brands and the areas they deliver from; several brands from one kitchen | Commercial | Browse as a customer to spot multi-brand kitchens; then find the operating company | App terms; no scraping |
| Events: Gulfood, 15–19 March 2027, Dubai World Trade Centre and Dubai Exhibition Centre [S5]; GulfHost, 3–5 November 2026, Dubai World Trade Centre (dates from secondary listings; confirm) [S6] | Operators, chefs and suppliers in one place | Commercial | Visit, meet, collect cards with consent to follow up | Event terms |
| LinkedIn | Founders, operations directors, F&B cost controllers, kitchen managers | Commercial | Manual search or Sales Navigator; no automation | User agreement forbids scraping |
| Google Maps ("cloud kitchen Dubai", "central kitchen Al Quoz") | Kitchens and their reviews | Commercial | Manual research; reviews show late or wrong orders | Terms forbid bulk extraction |
| Partners: order middleware partners (we integrate with Deliverect), food safety consultants and PIC trainers, kitchen equipment suppliers, accounting and VAT firms serving restaurants, shared cloud-kitchen facility operators | Introductions to operators | Commercial | Referral arrangement | Agreement per partner |

## 4. Messages

### 4.1 Landing page

**Headline:** One kitchen screen. Every channel. Food cost every day.

**Sub-headline:** Orders, recipes, stock and aggregator payouts for UAE cloud kitchens and
delivery-first chains, in one system with your brand, hosted in the UAE.

**Three problems we hear about:**
1. One tablet per aggregator on the pass. Orders are re-keyed, or never recorded at all.
2. Food cost is known only at month end, from Excel recipes and irregular counts.
3. Payout statements arrive net of commissions, promotions and refunds, and nobody checks
   them line by line.

**Three outcomes:**
1. **One screen on the pass.** Aggregator, phone, walk-in and direct orders on one kitchen
   display, split by station, with prep targets and late-order alerts.
2. **Food cost you can act on.** Recipes and sub-recipes, stock counts, wastage with reasons,
   and theoretical vs actual food cost per item and kitchen.
3. **Payouts checked.** Import each aggregator's statement, match it to your orders, and see
   commissions, promotions, refunds and unmatched amounts.

**Also included:** a branded ordering site per brand (pickup and delivery, phone login,
online payment, promo codes) with delivery by your own riders, prices per channel, central
kitchen batches with labels, purchase orders and receiving with temperature checks, food
safety checklists and temperature logs, health card and PIC certificate expiry, counter
takeaway with daily closing, several legal entities with their own TRNs, migration from
your Excel files.

**Call to action:** See a 10-minute demo with a sample kitchen group.

**Footer facts:** hosted in the UAE · your own branded deployment · priced per outlet /
kitchen.

### 4.2 First email (to a named operations lead)

> **Subject:** Aggregator tablets and food cost at {Company}
>
> Hi {First name},
>
> I saw that {Company} runs {brands / kitchens seen on the aggregator apps or website}. Most
> kitchen groups we speak to in the UAE have a tablet per aggregator, recipes in Excel, and
> payout statements that nobody checks line by line.
>
> We built a system for exactly that: every channel on one kitchen screen, food cost per
> item and kitchen from your recipes and counts, and aggregator statements matched to your
> orders. Each brand also gets its own ordering site. It runs under your brand and is hosted
> in the UAE.
>
> Would a 10-minute demo with a sample kitchen group be useful? If not, reply "no" and I
> won't write again.
>
> {Name} · {Company} · {phone} · {website}
> You received this because {Company}'s details are published on {source}. Unsubscribe: {link}

### 4.3 LinkedIn connection note (under 300 characters)

> Hi {First name}, I work on software for UAE cloud kitchens (one kitchen screen for all
> channels, daily food cost and aggregator payout checks). I'd like to connect and share
> what we're building.

### 4.4 Follow-up (once, 5–7 working days later, same thread)

> Hi {First name}, a short follow-up. The part kitchen groups like most is the payout check:
> import the aggregator's statement, and every line is matched to an order, with
> commissions, refunds and missing amounts listed. Happy to show it in 10 minutes. If now is
> not the time, just say so.

No further follow-up without a reply.

## 5. Demo storyline (10 minutes)

Uses *Zaatar & Co. Kitchens* from [DEMO-DATA.md](../DEMO-DATA.md) §06. Tell it as the area
manager's day. Aggregators appear as "Aggregator A/B/C". The group's café is shown only as
counter takeaway; dine-in is not shown.

1. **08:00, the area manager logs in** and sees what needs action: items below par in two
   kitchens, a central kitchen batch expiring tomorrow, opening checklists done or late.
2. **12:30, lunch rush:** the kitchen display shows orders from Aggregators A, B and C and
   the direct site, split by station, with timers; a late ticket is highlighted. *Wow 1: one
   screen instead of four tablets.*
3. **The customer's phone:** a brand's ordering site, a promo code, online payment; the
   order lands on the same kitchen display and goes to the outlet's own rider.
4. **15:00, stock count:** the storekeeper counts key items; the variance is approved by the
   kitchen manager, not by the counter.
5. **Food cost report:** theoretical vs actual per kitchen; one kitchen is 4 points over,
   with the items behind it. *Wow 2: where the money leaks.*
6. **The accountant:** this week's payout statement from Aggregator B is imported; AED 1,240
   is unmatched and highlighted, with commissions and promotions per order. *Wow 3:
   deductions they never checked.*
7. **Close:** the outlet's daily closing with sales by channel and payment type.

## 6. Objections

| Objection | Answer |
|---|---|
| "We already use Deliverect." | Good: we integrate with it, under your own subscription. Orders flow in automatically and menu changes are pushed out. Without middleware, orders are entered on a quick-entry screen in seconds. |
| "We already have a POS (e.g. Foodics)." | Keep it if it runs your dine-in. Importing its sales is on the roadmap, so today food cost needs orders in our system. If your sales must stay in your POS, we are probably not for you yet. |
| "We have dine-in restaurants too." | Dine-in (tables, split bills, service charge, reservations) is on the roadmap and uses the same orders and kitchen display. Today we cover delivery, pickup and counter takeaway. |
| "Can you connect to the aggregators directly?" | Aggregators approve integrators, and middleware already holds those links [S7]; we work through Deliverect today. Grubtech and other middleware are on the roadmap. |
| "Our chefs won't do stock counts on a system." | Key items daily, everything weekly, on a phone or tablet. The counter never sees expected quantities, and the variance goes to the manager. |
| "Will it help with inspections?" | Opening and closing checklists, temperature logs with alerts, receiving checks, batch labels, and health card and PIC certificate expiry, kept per outlet and ready to show. It does not submit anything to FoodWatch. |
| "Is our data safe? Where is it?" | Each client has its own deployment, hosted in the UAE. Card data never touches the platform (hosted payment page). Every refund, discount, wastage entry and price change is logged. |
| "How long does it take to start?" | Brands, menus, recipes and stock items are imported from Excel; payout mapping is set up from your real statements; then each role is trained. |
| "What does it cost?" | Priced per outlet / kitchen, with plans by team size. We quote after the demo. |

## 7. What we can claim

| Claim | Capability (MVP) |
|---|---|
| All channels on one kitchen display with prep targets and late alerts | `SAL.ORD.01`, `FNB.KDS.01`, `FNB.KDS.02`, `FNB.AGG.01` |
| Aggregator orders and menu sync through the client's Deliverect subscription | `FNB.AGG.02`, `FNB.AGG.03` (dependency: client's subscription) |
| Several brands per kitchen, prices and commission per channel | `FNB.BRD.01`, `FNB.CHN.01`, `SAL.CAT.01` |
| Branded ordering site per brand: pickup and delivery areas, phone login, online payment, promo codes | `FNB.DTC.01`, `FNB.DTC.02`, `POR.SLF.01`, `NTF.MSG.03`, `FIN.PAY.02`, `SAL.DSC.01` |
| Delivery of direct orders by own riders, with proof and cash handover | `FNB.DTC.03`, `WRK.DSP.01`, `WRK.POD.01`, `FIN.CSH.01` |
| Aggregator payout statement import and reconciliation | `FNB.REC.01`, `FIN.REC.01`, `FIN.PAY.01` |
| Recipes, stock counts, wastage, theoretical vs actual food cost | `STK.BOM.01`, `STK.CNT.01`, `FNB.WST.01`, `FNB.CST.01` |
| Par levels, purchase orders, receiving with temperature check, lots and expiry, supplier bills | `STK.ROP.01`, `STK.PUR.01`, `STK.GRN.01`, `STK.LOT.01`, `FIN.PAB.01` |
| Central kitchen batches, prep labels, outlet requisitions | `FNB.PRD.01`, `FNB.LBL.01`, `STK.PRQ.01` |
| Allergens and calories per item | `FNB.ALG.01` |
| Food safety checklists, temperature logs, equipment tickets | `FNB.FSF.01`, `WRK.CHK.01`, `WRK.TKT.01` |
| Health card and PIC certificate expiry | `HRM.EMP.01`, `DOC.EXP.01` |
| Counter takeaway and daily closing (Z report) | `SAL.POS.01` |
| Receipts, several entities with their own TRNs, VAT summary | `FIN.INV.01`, `PLT.ENT.01`, `FIN.VAT.01` |
| Sales mix, channel profitability after commission, dashboards | `FNB.RPT.01`, `RPT.DSH.01` |
| Migration from Excel | `PLT.IMP.01` |

**On the roadmap only (do not promise dates):** dine-in (floor plan, table sessions, orders
by course, split bills, service charge and fee lines), table reservations, Grubtech and
other middleware, third-party couriers, loyalty points, meal-plan subscriptions, customer
feedback, WhatsApp, import of sales from another POS, accounting export, bank statement
import, planned maintenance and equipment register, shift attendance and payroll,
map-drawn delivery zones.

**Never claim:** direct aggregator API integration without middleware, lower aggregator
commissions, submission to Dubai Municipality FoodWatch or ADAFSA, compliance certification.

## Sources

Checked 2026-10-15. *Official* = government or legislation; others are secondary.

| # | Source | Type | Used for |
|---|---|---|---|
| S1 | Dubai Municipality, "FoodWatch FAQs", https://www.dm.gov.ae/wp-content/uploads/2020/11/Foodwatch_FAQs_Booklet-English.pdf | Official | FoodWatch platform |
| S2 | Gulf News, "Now you can check food safety ratings of Dubai restaurants before visiting them", https://gulfnews.com/uae/health/now-you-can-check-food-safety-ratings-of-dubai-restaurants-before-visiting-them-1.83787259 | Secondary | Public rating lookup in the Dubai Municipality app |
| S3 | u.ae, "National Economic Register", https://u.ae/en/information-and-services/business/important-digital-services/national-economic-register | Official | Licence and activity lookup |
| S4 | Dubai Media Office, "Dubai Chamber announces formation of Restaurants Business Group" (2020), https://mediaoffice.ae/en/news/2020/July/27-07/Dubai-Chamber-announces-formation-of-Restaurants-Business-Group | Official | Restaurants Business Group |
| S5 | Austrade, "Gulfood 2027", https://www.austrade.gov.au/en/events/gulfood-2027 | Secondary | Gulfood dates and venues |
| S6 | Expoassist, "GulfHost Dubai 2026", https://expoassist.net/event/gulfhost-dubai | Secondary | GulfHost dates |
| S7 | Deliverect, "Noon Food integration", https://www.deliverect.com/ar-ae/integrations/noon-food (as cited in [06-fnb.md](../verticals/06-fnb.md) S3) | Secondary | Aggregator integrations through middleware |
