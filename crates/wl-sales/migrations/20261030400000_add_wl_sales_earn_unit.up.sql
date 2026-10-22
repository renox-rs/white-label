-- The loyalty programme earns `earn_rate` points for every `earn_unit_aed`
-- dirhams of an eligible sale (05 Clinics: 1 point per AED 10, `SAL.RUL.07`;
-- the default stays 1 point per AED 1).
ALTER TABLE "wl_sales_loyalty_programmes" ADD COLUMN earn_unit_aed INTEGER NOT NULL DEFAULT 1;
