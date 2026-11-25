# Atrium

A calm, professional theme for appointment businesses — studios, clinics,
salons, therapists, coaches and consultants — built for Vuuut's
**Booking & Appointment Scheduling** tool.

- **Homepage** — a hero that leads straight to booking, the service menu with
  prices (by category), signature treatments, how booking works, the team,
  the journal and a Visit us block with directions.
- **Services** (`booking/services.hbs`) — every bookable service by category,
  opening hours and policies alongside, and the team.
- **A service** (`booking/service.hbs`) — the details beside Vuuut's booking
  panel: who, a month of free days, times, and the customer's details.
- **A person** (`booking/staff.hbs`), **the customer's booking**
  (`booking/manage.hbs`, a ticket with calendar links, reschedule and
  cancel) and **reschedule** (`booking/reschedule.hbs`).

Data: `{{#get "services"}}`, `{{#get "service_categories"}}` and
`{{#get "booking_staff"}}`, plus each booking template's context (see
`crates/vuuut-front/src/booking.rs`). Theme settings hold the booking
address, the hero, and the address, hours and phone for Visit us.

Fonts: Instrument Serif and Manrope (SIL Open Font License 1.1), self-hosted.
