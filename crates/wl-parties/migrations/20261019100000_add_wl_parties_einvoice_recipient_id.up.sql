-- The company's e-invoicing recipient identifier (Peppol participant ID,
-- scheme + identifier, trimmed, as entered; wl-parties §3, issue #43).
-- NULL until entered or looked up by TRN. Persons have no company details row.
ALTER TABLE "wl_parties_company_details" ADD COLUMN einvoice_recipient_id TEXT;
