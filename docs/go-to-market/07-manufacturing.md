# 07 — Manufacturing: Go-to-Market Pack

> Status: draft · Last updated: 2026-10-15 · Source:
> [07-manufacturing.md](../verticals/07-manufacturing.md) (accepted)

## 1. Positioning

**One line:** Orders, work orders, quality checks, lot traceability and dispatch for UAE
manufacturers, in one branded light ERP instead of Excel, paper job cards and binders.

**For:** small and mid-sized manufacturers that make to order for B2B customers:
packaging, labels, plastic and metal parts, furniture, LED fittings, electrical panels.
One plant (sometimes two), 1–8 production lines.

**Not for (say so early):**
- Plants that need MRP optimisation or finite-capacity scheduling. The schedule board is
  placed by hand.
- Plants that want machine counts from PLCs, MES or SCADA. Not integrated and not planned.
- Companies looking for a full ERP with general ledger and financial statements. The books
  stay in their accounting software.
- Suppliers whose large retail customers require EDI orders.
- Companies that need PLM or an ISO document control system.
- Large plants already on SAP, Oracle or another full ERP.

## 2. Ideal client profile

| | Fits | Weak fit |
|---|---|---|
| Size | 20–250 staff, 1–8 production lines | under 15 staff, or several large plants |
| Products | 50–2,000 active items, made to order (some to stock) | continuous process plants |
| Customers | 30–300 B2B customers: FMCG, construction, retail, hospitality, oil and gas suppliers | mostly retail consumers |
| Location | industrial areas and free zones across the UAE, mainland or free zone | — |
| Payment practice | 30–120 day terms, often post-dated cheques | cash sales |
| Today | quotations and orders in Excel or Tally/Zoho/QuickBooks, paper job cards, plan on a whiteboard, QC records in binders, orders by email and WhatsApp | already on a full ERP and happy with it |

**Signals that a company fits**
- Holds a valid ICV certificate (listed in MoIAT's National ICV open data [S1]): it sells to
  customers who weigh ICV in tenders and ask for documents.
- Job ads for a production planner, QC inspector, storekeeper or "costing accountant"
  (manual work we remove).
- Makes regulated or food-contact products that need conformity certificates, or packaging
  affected by the single-use plastics ban.
- Customers in food, FMCG or oil and gas that ask for certificates of analysis per lot.
- Exhibits at manufacturing or packaging shows (§3).

**Who decides and who uses it**

| Role | Cares about | Message angle |
|---|---|---|
| Owner / general manager | Margin per order, cash, on-time delivery | Standard vs actual cost per work order; order-to-cash on one timeline |
| Production manager / planner | What to make, on which line, with which material | Work orders from sales orders, material shortages from the BOM, a schedule board per line |
| QC manager | Inspections, NCRs, certificates, complaints | Checks at the machine on a tablet; trace any lot in seconds |
| Accountant / credit control | Overdue customers, cheques, supplier bills | Orders held automatically when credit is exceeded; bills matched to PO and delivery |
| Sales | Repeat orders, order status questions | Customers reorder and track orders in a branded portal |

## 3. Lead sources

Build the list by hand: look up each company, check its website and LinkedIn, and record
where it came from. Do not scrape or bulk-copy any of the sources below.

| Source | What it gives | Type | How to use | Terms |
|---|---|---|---|---|
| MoIAT, National ICV open data, "ICV Certified Companies" ([page](https://moiat.gov.ae/en/open-data/national-icv)) [S1] | Companies with a valid ICV certificate: name, address, category; searchable by name or category | Official | Search a manufacturing category and look companies up one by one; use only business contact details | MoIAT open data may be reused, also commercially, under the UAE Federal Open Data Licence with attribution [S2]; we still build the list by hand and respect PDPL for named people |
| National Economic Register ([ner.economy.ae](https://ner.economy.ae)) [S3] | Licensed companies in all emirates; search by name or browse by activity (manufacturing activities) | Official | Confirm a company's licence and manufacturing activity | Terms not checked |
| MoIAT Industrial Registry [S4] | Every plant registers and updates yearly | Official | No public search was found; ask a prospect whether it is registered | Not applicable |
| LinkedIn | People with titles like plant manager, production manager, QC manager, operations director | Commercial | Manual search or Sales Navigator; no automation | User agreement forbids scraping |
| Google Maps ("packaging manufacturer Sharjah", "panel builder Dubai Industrial City") | Factories with addresses and reviews | Commercial | Manual research; check the website for products and customers | Terms forbid bulk extraction |
| Events: Gulfood Manufacturing (Dubai World Trade Centre, 3–5 November 2026) [S5]; Gulf Print & Pack (Dubai World Trade Centre, 12–14 January 2027) [S6]; Make it in the Emirates (ADNEC, Abu Dhabi; the 2026 edition ran 4–7 May, 2027 dates not yet announced) [S7] | Manufacturers, packaging and print converters, and their suppliers | Commercial | Visit, meet, collect cards with consent to follow up | Event terms |
| Partners: ICV certifying bodies and audit firms, machinery and raw material suppliers, accounting and VAT firms serving manufacturers | Introductions to manufacturers | Commercial | Referral arrangement | Agreement per partner |

## 4. Messages

### 4.1 Landing page

**Headline:** Know what each order really cost, and where every lot went.

**Sub-headline:** Orders, production, quality, traceability and dispatch for UAE
manufacturers, in one light ERP with your brand, hosted in the UAE.

**Three problems we hear about:**
1. Job cards are on paper and the plan is on a whiteboard, so nobody can say what a job
   really cost or when an order will ship.
2. When a customer complains, finding which raw material lot went into which delivery
   means digging through binders.
3. Customers order by email and WhatsApp and phone to ask for status, invoices and
   certificates.

**Three outcomes:**
1. **The shop floor on a tablet.** Work orders from sales orders, placed on a schedule
   board per line; materials issued with lot numbers, output and scrap reported at the
   line; standard vs actual cost per work order.
2. **Quality recorded at the machine.** Inspection checklists with photos; a failed check
   opens a non-conformance report; lots released only by QC; certificates of analysis or
   conformity per lot; trace any lot back to the supplier's lot and forward to customers.
3. **Customers help themselves.** A branded portal to reorder from a previous order or
   upload a PO, follow status, and download delivery notes, invoices and certificates.

**Also included:** customer price lists, quotations, credit limits with automatic credit
hold, BOMs with versions and material shortages, purchase orders in AED or foreign
currency with landed cost, goods received with supplier lots, supplier bills with 3-way
match, dispatch with delivery notes and proof of delivery, standard and zero-rated
invoices, cheques and receivables, ICV and conformity certificates kept with expiry
reminders, regulated and export-only items guarded at dispatch, breakdown tickets from the
line, migration from your Excel files.

**Call to action:** See a 10-minute demo with a sample packaging plant.

**Footer facts:** hosted in the UAE · your own branded deployment · priced per production
line or per plant.

### 4.2 First email (to a named manager)

> **Subject:** Job cards and traceability at {Company}
>
> Hi {First name},
>
> I saw that {Company} makes {products seen on their site} for {customer sectors}. Most
> manufacturers we speak to in the UAE still run job cards on paper and keep QC records in
> binders, so cost per order and lot traceability take days to piece together.
>
> We built a light ERP for exactly that: work orders and output reported on line tablets,
> inspections and NCRs at the machine, lot traceability from supplier to customer, and a
> portal where customers reorder and download their documents. It runs under your brand
> and is hosted in the UAE.
>
> Would a 10-minute demo with a sample packaging plant be useful? If not, reply "no" and I
> won't write again.
>
> {Name} · {Company} · {phone} · {website}
> You received this because {Company}'s details are published on {source}. Unsubscribe: {link}

### 4.3 LinkedIn connection note (under 300 characters)

> Hi {First name}, I work on software for UAE manufacturers (work orders, quality checks
> and lot traceability in one place). I'd like to connect and share what we're building.

### 4.4 Follow-up (once, 5–7 working days later, same thread)

> Hi {First name}, a short follow-up. The part plant managers like most is traceability:
> pick a finished pallet and see, in seconds, the supplier's raw material lot it came from
> and every customer who received the same lot. Happy to show it in 10 minutes. If now is
> not the time, just say so.

No further follow-up without a reply.

## 5. Demo storyline (10 minutes)

Uses *Emirates Pack Industries LLC* from [DEMO-DATA.md](../DEMO-DATA.md) §07. Tell it as
the general manager's day.

1. **08:00, the general manager logs in** and sees what needs action: a customer on credit
   hold with a new order, 2 NCRs open on one supplier's board, a raw material lot expiring,
   an overdue purchase order. Output vs plan on the dashboard.
2. **The customer's buyer** places a repeat order in the portal; sales confirms it against
   the customer's price list.
3. **Credit hold:** the order is held because the customer is over its limit; the
   accountant releases it.
4. **The line tablet:** the work order is placed on the line's schedule and issued; the
   operator reports output and scrap.
5. **QC at the machine:** a failed measurement on the checklist opens an NCR. *Wow 1:
   quality issues recorded at the machine, not on paper.*
6. **A customer complaint:** trace a finished pallet back to the supplier's board lot and
   forward to every customer who received it. *Wow 2: answer a complaint in seconds.*
7. **Close:** dispatch with delivery note and proof of delivery; the invoice is issued.
   *Wow 3: order to cash on one timeline.*

## 6. Objections

| Objection | Answer |
|---|---|
| "We already use Tally / Zoho / QuickBooks." | Keep it for the books; we do not replace the general ledger. We cover orders, production, quality, stock and invoicing, and give your accountant a VAT summary; accounting export is on the roadmap. |
| "We need automatic production planning." | The schedule board is placed by hand per line, with material shortages from the BOM. If planning optimisation is your main need, we are probably not for you. |
| "Can it take counts from our machines?" | No. Output is reported on line tablets. Machine, PLC and MES integration is not part of the product. |
| "Our shop-floor Wi-Fi is patchy." | The tablets need a connection. When Wi-Fi drops, operators use paper job cards that the supervisor enters at the end of the shift. An offline mode is on the roadmap. |
| "Customers ask for our ICV certificate." | The certificate is kept with an expiry reminder, attached to quotations and shown in the customer portal. Preparing the ICV data pack for the certifying body is on the roadmap. |
| "Some of our products need ECAS or food-contact certificates." | Items are flagged as regulated, and a regulated item is dispatched only while a valid certificate is on file; export-only items cannot be sold for domestic delivery. A full conformity register is on the roadmap. |
| "What about machine maintenance?" | Breakdowns are raised as tickets from the line tablet and closed with photos. A machine register, planned maintenance and downtime reports are on the roadmap. |
| "How do you cost labour?" | An hourly rate per line covering labour and overhead, times standard hours on the BOM and the work order's actual hours. Labour booking per employee is on the roadmap. |
| "What does it cost?" | Priced per production line or per plant, with plans by size. We quote after the demo. |

## 7. What we can claim

| Claim | Capability (MVP) |
|---|---|
| Customer price lists, quotations, sales orders | `SAL.CAT.01`, `FIN.QUO.01`, `SAL.ORD.01`, `DOC.GEN.01` |
| Credit limits, credit check and credit hold | `SAL.CRD.01`, `PLT.APR.01` |
| BOMs with versions and material shortages | `STK.BOM.01`, `STK.LOC.01` |
| Work orders from sales orders or for stock; schedule board per line | `MFG.PRD.01`, `MFG.PLN.01` |
| Material issue, output and scrap reported on line tablets | `MFG.PRD.02`, `STK.MOV.01` |
| QC plans, inspections on tablets, NCRs, lot release, certificates per lot | `MFG.QCL.01`, `WRK.CHK.01`, `MFG.QCL.02`, `MFG.QCL.03` |
| Lots and traceability from supplier to customer | `STK.LOT.01`, `MFG.TRC.01` |
| Purchasing in AED or foreign currency, landed cost, goods received with supplier lots | `STK.PRQ.01`, `STK.PUR.01`, `FIN.CUR.01`, `STK.LND.01`, `STK.GRN.01` |
| Supplier bills with 3-way match | `FIN.PAB.01` |
| Dispatch with delivery notes and proof of delivery | `STK.DSP.01`, `WRK.DSP.01`, `WRK.POD.01` |
| Standard and zero-rated invoices, cheques, receivables | `FIN.INV.01`, `FIN.CHQ.01`, `FIN.REC.01` |
| Standard vs actual cost per work order | `MFG.CST.01`, `FIN.CST.01` |
| Customer portal: status, documents, reorder or PO upload | `POR.SLF.01` |
| ICV and conformity certificates with expiry reminders; regulated and export-only flags | `DOC.FIL.01`, `DOC.EXP.01` |
| Breakdown tickets from the line | `WRK.TKT.01`, `WRK.TKT.03` |
| Production reports and dashboards | `MFG.RPT.01`, `RPT.DSH.01` |
| VAT summary per entity | `FIN.VAT.01` |
| Migration from Excel | `PLT.IMP.01` |

**On the roadmap only (do not promise dates):** routings with standard times per
operation, shift attendance and labour booking, machine register, planned maintenance,
downtime and OEE, customer returns linked to lots, ICV data pack, conformity register,
supplier portal, catalogue ordering in the portal, offline mode for line tablets, weighing
scales, direct label printers, WhatsApp, accounting export, bank statement matching,
expense claims, leads pipeline.

**Never claim:** MRP optimisation or finite-capacity scheduling, MES/PLC/SCADA integration,
a general ledger or financial statements, EDI, PLM, ISO document control, that the system
calculates an ICV score or issues certificates.

## Sources

Checked 2026-10-15. *Official* = government or legislation; others are secondary.

| # | Source | Type | Used for |
|---|---|---|---|
| S1 | MoIAT, "National ICV" open data, https://moiat.gov.ae/en/open-data/national-icv | Official | ICV certified companies list |
| S2 | MoIAT, "Open Data Policy", https://moiat.gov.ae/en/open-data/open-data-policy | Official | Reuse terms (UAE Federal Open Data Licence) |
| S3 | u.ae, "National Economic Register", https://u.ae/en/information-and-services/business/important-digital-services/national-economic-register | Official | Licence and activity lookup |
| S4 | MoIAT, "Industrial Registry", https://moiat.gov.ae/en/industrial-registry | Official | Yearly plant registration |
| S5 | Gulfood Manufacturing, https://www.gulfoodmanufacturing.com/ | Secondary | Gulfood Manufacturing 2026 dates |
| S6 | The Packaging Portal, "Gulf Print & Pack announces January 2027 dates", https://www.thepackagingportal.com/industry-news/gulf-print-pack-announces-january-2027-dates/ | Secondary | Gulf Print & Pack 2027 dates |
| S7 | ADNEC, "Make It In the Emirates", https://www.adnec.ae/en/eventlisting/make-it-in-the-emirates-1 | Secondary | Make it in the Emirates 2026 dates and venue |
