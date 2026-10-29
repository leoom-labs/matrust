import { functions } from '../../lib/functions';
export function GET() {
  return new Response(JSON.stringify(functions.map(({name,summary}) => ({name,summary}))), { headers: { 'Content-Type': 'application/json; charset=utf-8' } });
}
