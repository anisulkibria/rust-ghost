# Plinth

A free storefront theme for Vuuut, made for furniture and home goods. The
homepage is built from your store (Tools → E-commerce): collections become
the categories, hero slides and banners; products fill the carousels, picks,
offers and deals. Anything without a photo is drawn, so a new store looks
finished before its photography is done.

## How the homepage reads your store

| Block | Comes from |
| --- | --- |
| Category circles, **Shop by category**, the header's category menu and search | published collections, in their admin order |
| Hero slides | the first three collections: title, description (the line above the title), image |
| New arrivals | the ten newest active products |
| Flash sale band | Design → `sale_ends` and `sale_text`; hidden when `sale_ends` is empty or past |
| Four tall picks; Shop the look hotspots | products marked **Feature on the store's home** |
| Featured collection | the first collection that has products, with four of its pieces |
| Get inspired by spaces | your newest posts |
| Two banners | the fourth and fifth collections |
| Shop our offers | products with a compare-at price above their price |
| The studio | Design → `about_title`, `about_text`, the site cover image, and a link to the page with slug `about` |
| Yellow quote band | Design → `quote`, `quote_name`; hidden when empty |
| Deal block | the five lowest-priced products in stock, titled by `deal_title` |
| Instagram band | Design → `gram_handle`; hidden when empty |
| Contact strip | Design → `phone`, `contact_email`, `address`; each item only when set |

A block with nothing to show is left out.

## Products

| Product field | Where it shows |
| --- | --- |
| Images | the first fills the card; the second appears on hover |
| Product type | the small label above the name; also picks the drawing |
| Tag `shape:chair` | which drawing to use when there is no photo: `chair`, `armchair`, `sofa`, `table`, `desk`, `lamp`, `stool`, `bench`, `storage`, `sideboard`, `bed`, `vase` |
| An option named `Color`, `Colour` or `Finish` | swatches on the card and swatch pills on the product page. CSS colour names work, and so do common materials: oak, walnut, ash, natural, sand, sage, olive, tan, brass, terracotta, ivory, linen, navy… |
| Compare-at price | the struck-through price and a `−24%` badge |
| A single variant in stock | an **Add to cart** button right on the card |

## Theme settings (Settings → Design)

`announcement`, `sale_url`, `promo_title`, `sale_ends`, `sale_text`,
`deal_title`, `about_title`, `about_text`, `quote`, `quote_name`, `values`
(four promises separated by `|`), `gram_handle`, `phone`, `contact_email`,
`address`.

Navigation: the primary menu sits in the header after **Shop all**; the
secondary menu fills the top bar's left side and the footer's Help column.

## Store templates

`shop/shop.hbs`, `shop/collection.hbs`, `shop/collections.hbs` and
`shop/product.hbs` replace the store's own pages; cart, checkout and the
customer dashboard keep the store's markup. `{{#get "collections"}}` and
`{{#get "products"}}` work in any template:

```hbs
{{#get "collections" limit="8" filter="product_count:>0" as |rooms|}}
  {{#foreach rooms}}<a href="{{url}}">{{title}} ({{product_count}})</a>{{/foreach}}
{{/get}}

{{#get "products" filter="collections:lighting+on_sale:true" order="price asc" limit="4"}}
  {{#foreach products}}{{title}} — {{lookup this "price"}}{{/foreach}}
{{/get}}
```

Product fields beyond the store's card data that this theme uses:
`save_percent`, `colors`, `variant_count`, `quick_add` (the variant id when
there is exactly one to buy) and `add_url`.

## Licence

MIT. Fonts: Familjen Grotesk, Hanken Grotesk and Newsreader (SIL Open Font
Licence), loaded from Google Fonts.
