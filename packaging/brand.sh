# shellcheck shell=bash
# The app's branding: APP_NAME, APP_SLUG, APP_ID, APP_VENDOR, APP_HOMEPAGE.
# Source this from a packaging script; it exports the values from branding/brand.env.
set -a
# shellcheck source=../branding/brand.env
. "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/branding/brand.env"
set +a

# brand_subst FILE OUT: expand the @APP_*@ placeholders in a template.
brand_subst() {
  sed -e "s/@APP_NAME@/$APP_NAME/g" \
      -e "s/@APP_SLUG@/$APP_SLUG/g" \
      -e "s/@APP_ID@/$APP_ID/g" \
      -e "s/@APP_VENDOR@/$APP_VENDOR/g" \
      -e "s/@APP_HOMEPAGE@/$APP_HOMEPAGE/g" \
      "$1" >"$2"
}
