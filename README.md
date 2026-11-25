# Elías Castro Soto

# Apartado de productos de feria (Stellar / Soroban)

Contrato inteligente tipo **escrow** para que un comprador pueda apartar productos de un
agricultor pagando por adelantado, sin que ninguna de las dos partes tenga que confiar a ciegas en la otra.

> Tarea 2 del bootcamp: primer contrato inteligente en Stellar.

## El problema

En las ferias del agricultor de Costa Rica, comprar directamente al productor funciona por confianza:
muchas veces el cliente quiere apartar producto con anticipación (una canasta, una cantidad de
verdura, algo que se cosecha por pedido) y el productor quiere asegurarse de que el pedido es en serio.

Hoy se resuelve con un mensaje de WhatsApp y un adelanto, o sin adelanto. Eso deja dos riesgos:

- Si el comprador paga por adelantado y el productor no llega, **el comprador pierde su dinero** o tiene que reclamarlo a mano.
- Si el productor aparta y guarda el producto sin adelanto, **el comprador puede no aparecer** y el productor pierde la venta.

## La solución

Un "sobre de manila digital": el comprador deposita el pago en el contrato. El dinero no es de
ninguna de las partes mientras el pedido está pendiente, y solo se mueve según reglas que nadie
puede cambiar.

| Función | Quién la firma | Qué hace |
|---|---|---|
| `reservar(comprador, productor, token, monto, fecha_limite)` | comprador | Deposita el monto en el contrato y devuelve el `id` de la reserva |
| `marcar_entregado(id)` | productor | Declara que entregó, solo hasta la fecha límite |
| `confirmar_recepcion(id)` | comprador | Libera el pago al productor |
| `reclamar_pago(id)` | productor | Cobra si el comprador no confirmó pasadas 48 h desde la entrega |
| `reembolsar(id)` | comprador | Recupera su dinero si venció la fecha límite y no hubo entrega |
| `obtener_reserva(id)` | cualquiera | Consulta (solo lectura) |

El punto clave del diseño es que **ninguna parte puede quedarse con el dinero y el producto a la vez**:
el comprador no puede recibir el producto y pedir el reembolso (porque el productor marcó la entrega
y puede reclamar el pago), y el productor no puede cobrar sin marcar la entrega a tiempo.

### Por qué elegí este problema

Es pequeño, cotidiano en Costa Rica y se explica en dos líneas. Además usa justo lo que enseña un
escrow: fondos en un lugar neutral que se liberan por condiciones. El productor solo tiene que
aceptar el encargo y entregarlo; la parte técnica la usa el comprador.

## Seguridad

- **Autenticación con `require_auth()`** en todas las funciones que mueven fondos o cambian estado, sobre la dirección guardada en la reserva (nunca una comparación manual de direcciones).
- **Sin administrador ni llave maestra**: nadie puede retirar fondos fuera de las reglas.
- **Máquina de estados estricta**: cada función verifica el estado actual, así que no hay doble pago ni doble reembolso.
- **Estado antes que transferencia**: el estado se actualiza y guarda antes de mover los fondos.
- **Montos enteros (`i128`)**, mayores a 0. No hay números decimales ni cálculos de porcentajes.
- **Sumas con `checked_add`** para el contador y los plazos.
- **IDs asignados por el contrato**, así nadie puede elegir ni pisar la reserva de otro.
- **Errores propios y claros** (`#[contracterror]`), sin `unwrap()` sobre datos que pueden no existir.
- **TTL gestionado**: cada reserva extiende su TTL al guardarse, y el contador también.
- El comprador y el productor deben ser cuentas distintas.

### Limitaciones conocidas (honestas)

- **No hay arbitraje de disputas**: si el producto llega en mal estado, el contrato no lo sabe. Una versión futura podría agregar un árbitro.
- El productor necesita una **wallet de Stellar** para marcar la entrega y recibir el pago.
- El plazo de confirmación (48 h) es una constante del contrato, no un parámetro.
- Usa el **reloj del ledger**, así que los plazos tienen la precisión de la red, no del segundo exacto.
- El código no ha pasado una auditoría profesional. Esto es un proyecto de aprendizaje y no debe usarse con dinero real.

## Cómo correrlo

Requisitos: Rust reciente (el SDK 30 exige 1.91 o superior) y, para compilar a WASM, el target
`wasm32v1-none` y [Stellar CLI](https://developers.stellar.org/docs/tools/cli).

```bash
# Pruebas (no necesitan red ni wallets)
cargo test

# Compilar el contrato a WASM
stellar contract build
```

## Pruebas

`cargo test` ejecuta 20 pruebas:

- **Flujos exitosos**: pago completo al productor, reembolso, cobro por falta de confirmación, reservas independientes con ids secuenciales.
- **Validaciones**: monto 0 o negativo, fecha límite pasada, comprador igual a productor, reserva inexistente.
- **Estados y tiempos**: doble pago, doble reembolso, reembolso antes de la fecha límite (incluyendo el instante exacto), reembolso tras una entrega, entrega fuera de plazo, cobro antes del plazo.
- **Autorización**: un tercero o la contraparte intenta firmar por el comprador o el productor en cada función y falla sin mover fondos.

## Estructura

```
Cargo.toml                    # workspace
contracts/feria/
  Cargo.toml
  src/lib.rs                  # el contrato
  src/test.rs                 # las pruebas
```
