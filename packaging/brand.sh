# shellcheck shell=bash
# The app's branding: APP_NAME, APP_SLUG, APP_ID, APP_VENDOR, APP_HOMEPAGE.
# Source this from a packaging script; it exports the values from branding/brand.env.
set -a
# shellcheck source=../branding/brand.env
. "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/branding/brand.env"
set +a
