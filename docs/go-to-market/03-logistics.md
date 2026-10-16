# 03 — Logistics & Freight Forwarding: Go-to-Market Pack

> Status: draft · Last updated: 2026-10-15 · Source:
> [03-logistics.md](../verticals/03-logistics.md) (accepted)

## 1. Positioning

**One line:** Bookings, runs, proof of delivery, COD cash and billing for UAE courier,
trucking and freight companies, in one branded system instead of WhatsApp, paper run sheets
and a cash notebook.

**For:** last-mile couriers delivering for online shops (often cash on delivery), B2B
trucking companies working inside the UAE, and freight forwarders that also collect and
deliver; with 20–200 vehicles and their own drivers.

**Not for (say so early):**
- Companies that need live vehicle tracking on a map from day one. Location is captured at
  each scan and delivery; telematics is on the roadmap.
- Warehouse operators whose core is stock per client SKU, bin locations and pick and pack.
  Storage can be billed, but warehouse management is not included.
- Platforms that work mostly through owner-drivers or subcontracted carriers.
- Companies whose clients must push orders from their online store by API from the start.
- Pure customs brokers. Declarations are filed in Mirsal or ATLP as today; we only keep the
  references.
- Very small operators with a few vans, who are better served by a spreadsheet.

## 2. Ideal client profile

| | Fits | Weak fit |
|---|---|---|
| Fleet | 20–200 vehicles (vans, pick-ups, trucks, motorbikes for express) | under 10 vehicles |
| Team | 30–400 drivers and helpers, 1–6 hubs or depots | — |
| Clients | 5–200 B2B clients: online shops, distributors, importers | mostly walk-in consumers |
| Location | Dubai and Abu Dhabi hubs; deliveries across the UAE | — |
| Model | own drivers, COD common, monthly or weekly billing from a rate card | mostly subcontracted fleet |
| Today | orders by email, WhatsApp or Excel; printed run sheets; COD counted in a notebook; vehicle documents in a spreadsheet; Tally/Zoho/QuickBooks for the books | already on a courier platform and happy with it |

**Signals that a company fits**
- Job ads for a dispatcher, COD cashier, fleet coordinator or "operations executive" (manual
  work we remove).
- Online shops name it as their delivery partner, or recipients complain online about
  missed deliveries and calls.
- A member of the Dubai Delivery Business Group [S4] or of NAFL [S5].
- A new hub, a new large online-shop client, or a freight desk added to a courier business.

**Who decides and who uses it**

| Role | Cares about | Message angle |
|---|---|---|
| Owner / general manager | COD money, client retention, cost per drop | Every dirham of COD traced from the recipient to the client's remittance |
| Operations manager | First-attempt success, reattempts, driver issues | Runs built in one place; failed attempts with reasons and reattempts scheduled |
| Accountant | Invoicing thousands of deliveries, remittances, fines | One invoice per client from the rate card; remittances prepared and approved by two people |
| Fleet manager | Registrations, insurance, services, fines and tolls | Expiry reminders, services by kilometres, Salik/Darb and fines allocated to driver or job |
| Customer service | "Where is my shipment?" calls | Clients and recipients see the tracking timeline themselves |

## 3. Lead sources

Build the list by hand: look up each company, check its website and LinkedIn, and record
where it came from. Do not scrape or bulk-copy any of the sources below.

| Source | What it gives | Type | How to use | Terms |
|---|---|---|---|---|
| National Economic Register ([ner.economy.ae](https://ner.economy.ae)) [S1] | Licensed companies in all emirates; search by name or browse by activity (e.g. courier, freight, road transport) | Official | Confirm a company's licence and activity | Terms not checked |
| Emirates Post Group courier licensing [S2] | Courier companies must hold a postal activity licence from EPG | Official | No public list of licensees was found; ask a prospect whether it holds the licence (it signals a real courier business) | Not applicable |
| Dubai Chamber of Commerce, Dubai Delivery Business Group (launched July 2025) [S3][S4] | Delivery companies active in Dubai | Semi-official | Join and attend group meetings | Membership |
| National Association of Freight & Logistics (NAFL), UAE representative in FIATA [S5] | Freight forwarders and logistics companies | Semi-official | Attend its events; no public member list was found | Membership |
| LinkedIn | People with titles like operations manager, last-mile manager, fleet manager, freight forwarding manager | Commercial | Manual search or Sales Navigator; no automation | User agreement forbids scraping |
| Google Maps ("courier company Dubai", "freight forwarder Jebel Ali") | Local offices with reviews | Commercial | Manual research; reviews show delivery pain points | Terms forbid bulk extraction |
| Online shops' delivery pages | Which couriers shops use and offer COD with | Commercial | Note courier names seen on shops' checkout and FAQ pages | Site terms; no scraping |
| Events: Breakbulk Middle East (Dubai World Trade Centre, 2–3 February 2027) [S6]; Seamless Middle East (Dubai, e-commerce and retail; 2027 dates not yet confirmed) [S7] | Forwarders, hauliers, online sellers and their couriers | Commercial | Visit, meet, collect cards with consent to follow up | Event terms |
| Partners: e-commerce agencies and store builders, SMS providers, accounting and VAT firms serving transport companies | Introductions to couriers and forwarders | Commercial | Referral arrangement | Agreement per partner |

## 4. Messages

### 4.1 Landing page

**Headline:** Every parcel and every dirham of COD, accounted for.

**Sub-headline:** Bookings, runs, proof of delivery, cash on delivery and billing for UAE
couriers and freight companies, in one system with your brand, hosted in the UAE.

**Three problems we hear about:**
1. COD cash is counted in a notebook at the end of the day. Shortages and late remittances
   are found weeks later.
2. Clients phone to ask where their shipment is, and the answer depends on calling the
   driver.
3. Billing means exporting thousands of deliveries to Excel and pricing them by hand.

**Three outcomes:**
1. **COD you can trace.** Collected at the door, handed over at the end of the day and
   counted by a second person, shortages flagged, and remitted to each client with a
   statement prepared and approved by two people.
2. **Clients and recipients help themselves.** A branded client portal to book by CSV, print
   labels and see proof of delivery; recipients get an SMS with a tracking link.
3. **Billing from the rate card.** Each client's deliveries priced by zone and weight, trip,
   hour or pallet, with COD fees and surcharges, in one invoice.

**Also included:** hub scanning with the phone camera, runs and dispatch, a driver app on
company or own phones with photo, signature or OTP, failed attempts and returns, freight
jobs with documents, customs references and disbursements, job costing, multi-currency,
standard and zero-rated VAT per line, vehicle documents with expiry reminders, services by
date or kilometres, daily vehicle checks, Salik/Darb tolls and fines allocated to driver or
job, warnings for rider runs in the midday break, migration from your Excel files.

**Call to action:** See a 10-minute demo with a sample courier company.

**Footer facts:** hosted in the UAE · your own branded deployment · priced per shipments per
month or per vehicle.

### 4.2 First email (to a named manager)

> **Subject:** COD and run sheets at {Company}
>
> Hi {First name},
>
> I saw that {Company} delivers for {online shops / sectors seen on their site}. Most
> couriers we speak to in the UAE still print run sheets, count COD in a notebook and
> answer "where is my parcel?" calls by phoning the driver.
>
> We built a system for exactly that: a driver app with proof of delivery, a cash handover
> counted by a second person, a client portal with tracking, and invoices built from each
> client's rate card. It runs under your brand and is hosted in the UAE.
>
> Would a 10-minute demo with a sample courier be useful? If not, reply "no" and I won't
> write again.
>
> {Name} · {Company} · {phone} · {website}
> You received this because {Company}'s details are published on {source}. Unsubscribe: {link}

### 4.3 LinkedIn connection note (under 300 characters)

> Hi {First name}, I work on software for UAE couriers and freight companies (runs, proof
> of delivery, COD and billing in one place). I'd like to connect and share what we're
> building.

### 4.4 Follow-up (once, 5–7 working days later, same thread)

> Hi {First name}, a short follow-up. The part operations managers like most is the end of
> the day: each driver's COD is counted by a second person, any shortage is flagged, and
> the client's remittance statement is built from it. Happy to show it in 10 minutes. If
> now is not the time, just say so.

No further follow-up without a reply.

## 5. Demo storyline (10 minutes)

Uses *Najm Express Cargo LLC* from [DEMO-DATA.md](../DEMO-DATA.md) §03. Tell it as the
operations manager's day.

1. **08:00, the operations manager logs in** and sees what needs action: 40 shipments
   awaiting reattempt, 2 drivers with cash shortages at yesterday's handover, 1 COD
   remittance overdue to a client, 3 vehicle registrations expiring, a van past its service
   kilometres.
2. **An online shop's portal:** the client uploads 50 shipments by CSV and prints labels.
3. **Dispatch:** the dispatcher builds runs by area and assigns drivers and vans; the
   driver's phone shows the day's stops.
4. **At the door:** the driver records delivery with photo, signature and location; it
   appears at once in the client portal. *Wow 1.*
5. **A failed attempt:** reason recorded, reattempt scheduled, the recipient notified by
   SMS with the tracking link.
6. **End of day:** the driver hands over COD cash, counted by a cashier; a shortage is
   flagged for approval. *Wow 2: COD money they can finally trace.*
7. **Close:** the client's invoice for the period from the rate card, with COD fees and
   surcharges. *Wow 3: one invoice from 6,000 transactions.*

## 6. Objections

| Objection | Answer |
|---|---|
| "Can we see our vans live on a map?" | Not today. Location is captured at every scan and delivery, and clients see the shipment's place in the run. Live tracking from telematics devices is on the roadmap. |
| "Our clients want to send orders from Shopify / their store." | Today clients book in the portal or upload a CSV. A client API and store plugins are on the roadmap. If that is a must on day one, we are probably not for you yet. |
| "Does it optimise routes?" | Not yet; the dispatcher groups shipments by area and sets the stop order. Route optimisation is on the roadmap. |
| "Our drivers use their own phones." | That works. The driver app runs in the browser on any phone; location is taken only at scans and deliveries, and the recipient's details disappear after the stop. |
| "We also run a warehouse." | Storage is billed per pallet or CBM per day from the rate card. Warehouse management (bins, stock per client, pick and pack) is on the roadmap; if it is your core business, we are probably not for you. |
| "Most of our fleet is subcontracted." | Runs for subcontracted carriers and owner-drivers are on the roadmap. Today the system fits companies with their own drivers. |
| "Does it file our customs declarations?" | No. You or your clearing agent file in Mirsal or ATLP as today; the freight job keeps the declaration number, duties, delivery order and disbursements. |
| "We already use Tally / Zoho / QuickBooks." | Keep it for the books. We cover shipments, COD, fleet and invoicing, and give your accountant a VAT summary; accounting export is on the roadmap. |
| "What does it cost?" | Priced per shipments per month or per vehicle, with plans by size. We quote after the demo. |

## 7. What we can claim

| Claim | Capability (MVP) |
|---|---|
| Shipment booking in the client portal or by CSV; labels and waybills | `LOG.SHP.01`, `LOG.SHP.02`, `POR.SLF.01`, `DOC.GEN.01` |
| Hub scanning with the phone camera | `LOG.HUB.01`, `PLT.SCN.01` |
| Runs and dispatch to driver and vehicle | `LOG.RTE.01`, `WRK.DSP.01` |
| Driver app with photo, signature or OTP and location; failed attempts and returns | `WRK.POD.01` |
| Tracking timeline, public tracking link, recipient SMS | `LOG.TRK.01`, `POR.SLF.02`, `NTF.MSG.03` |
| COD collection, end-of-day cash handover, remittances to clients | `LOG.COD.01`, `FIN.CSH.01` |
| Rate cards and invoices from many deliveries | `FIN.RAT.01`, `FIN.USG.01`, `FIN.INV.01` |
| Freight jobs with quotations, documents, customs references, disbursements, job costing, multi-currency | `FIN.QUO.01`, `LOG.JOB.01`, `LOG.CUS.01`, `FIN.DSB.01`, `FIN.CST.01`, `FIN.CUR.01` |
| Standard and zero-rated VAT per line; VAT summary per entity | `FIN.INV.01`, `FIN.VAT.01` |
| Vehicles with documents and expiry reminders, services, daily checks | `STK.AST.01`, `LOG.FLT.01`, `DOC.EXP.01`, `WRK.PPM.01`, `WRK.CHK.01` |
| Fines and tolls imported and allocated to driver, vehicle or job | `LOG.FIN.01` |
| Driver records and shift check-in | `HRM.EMP.01`, `HRM.ATT.01` |
| Tickets for breakdowns and complaints | `WRK.TKT.01`, `WRK.TKT.03` |
| Cheques and receivables | `FIN.CHQ.01`, `FIN.REC.01` |
| Operations reports and dashboards | `LOG.RPT.01`, `RPT.DSH.01` |
| Migration from Excel | `PLT.IMP.01` |

**On the roadmap only (do not promise dates):** live vehicle tracking from telematics,
route optimisation, client API and e-commerce plugins, outbound webhooks, WhatsApp,
recipient delivery-slot choice, online and card-on-delivery payment, claims valuation,
subcontracted carriers, address geocoding, warehouse management, customer feedback, leads
pipeline, accounting export, bank statement matching, customs system integration.

**Never claim:** live tracking from drivers' phones in the background, that the system settles whether you need a courier licence or a licence
to collect COD (both open legal questions).

## Sources

Checked 2026-10-15. *Official* = government or legislation; others are secondary.

| # | Source | Type | Used for |
|---|---|---|---|
| S1 | u.ae, "National Economic Register", https://u.ae/en/information-and-services/business/important-digital-services/national-economic-register | Official | Licence and activity lookup |
| S2 | Emirates Post Group, "Issue Postal Activity License", https://www.epgl.ae/issue-license | Official | Courier licensing |
| S3 | WAM, "Dubai Chamber of Commerce launches Dubai Delivery Business Group", https://www.wam.ae/en/article/bkm30rv-dubai-chamber-commerce-launches-dubai-delivery | Official | Delivery business group |
| S4 | Dubai Chamber of Commerce, "Dubai Chamber of Commerce launches Dubai Delivery Business Group", https://www.dubaichambercommerce.com/en/e/dcc_news_detailed_page/29577/3364440 | Semi-official | Delivery business group |
| S5 | Breakbulk Middle East, "National Association of Freight and Logistics (NAFL)", https://middleeast.breakbulk.com/Partner-List/national-association-of-freight-and-logistics; Air Cargo Update, "NAFL brings logistics sector to new heights", https://aircargoupdate.com/?p=2146 | Secondary | NAFL, FIATA representation |
| S6 | Breakbulk Middle East, https://middleeast.breakbulk.com/ | Secondary | Breakbulk Middle East 2027 dates |
| S7 | EventsEye, "Seamless Middle East", https://www.eventseye.com/fairs/f-seamless-middle-east-14803-1.html | Secondary | Seamless Middle East (2027 dates unconfirmed) |
