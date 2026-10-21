-- When the invoice of a reopening was paid again, so the package's reopened
-- balance can be the sum of what is still open per invoice (BL-SAL-16).
ALTER TABLE "wl_sales_package_reopenings" ADD COLUMN settled_at TEXT;
