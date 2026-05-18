# Railway notes

- Use one persistent service for `api`
- Use one persistent service for `daemon`
- Keep `daemon` private
- Mount daemon state volume at `/app/data`
- Public traffic should enter only through `api`

