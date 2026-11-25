#![no_std]

//! Apartado de productos de feria del agricultor.
//!
//! Un comprador paga por adelantado y el contrato retiene los fondos.
//! Si el productor entrega, cobra; si no entrega, el comprador recupera su dinero.
//! No hay administrador: ninguna cuenta puede mover fondos ajenos.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, token, Address, Env};

#[cfg(test)]
mod test;

/// Segundos que tiene el comprador para confirmar la recepción tras la entrega (48 h).
/// Pasado ese plazo, el productor puede reclamar el pago por su cuenta.
pub const PLAZO_CONFIRMACION: u64 = 172_800;

// Los ledgers de Stellar cierran cada ~5 s: 17_280 ledgers ≈ 1 día.
const DIA_EN_LEDGERS: u32 = 17_280;
const TTL_UMBRAL: u32 = 30 * DIA_EN_LEDGERS;
const TTL_EXTENDER: u32 = 60 * DIA_EN_LEDGERS;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Estado {
    Reservado,
    Entregado,
    Pagado,
    Reembolsado,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reserva {
    pub comprador: Address,
    pub productor: Address,
    pub token: Address,
    /// Monto en la unidad mínima del token (por ejemplo, stroops para XLM).
    pub monto: i128,
    /// Timestamp (segundos) hasta el cual el productor puede marcar la entrega.
    pub fecha_limite: u64,
    /// Timestamp en que se marcó la entrega (0 mientras no se haya entregado).
    pub entregado_en: u64,
    pub estado: Estado,
}

#[contracttype]
pub enum DataKey {
    ContadorReservas,
    Reserva(u64),
}

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Error {
    MontoInvalido = 1,
    FechaInvalida = 2,
    MismaPersona = 3,
    ReservaNoExiste = 4,
    EstadoInvalido = 5,
    FechaLimitePasada = 6,
    FechaLimiteNoAlcanzada = 7,
    PlazoNoCumplido = 8,
    Desbordamiento = 9,
}

#[contract]
pub struct FeriaContract;

#[contractimpl]
impl FeriaContract {
    /// El comprador aparta un producto: deposita `monto` en el contrato.
    /// Devuelve el id de la reserva.
    pub fn reservar(
        env: Env,
        comprador: Address,
        productor: Address,
        token: Address,
        monto: i128,
        fecha_limite: u64,
    ) -> Result<u64, Error> {
        comprador.require_auth();

        if monto <= 0 {
            return Err(Error::MontoInvalido);
        }
        if comprador == productor {
            return Err(Error::MismaPersona);
        }
        if fecha_limite <= env.ledger().timestamp() {
            return Err(Error::FechaInvalida);
        }

        // El id lo asigna el contrato: nadie puede elegir ni pisar un id ajeno.
        let id: u64 = env
            .storage()
            .instance()
            .get(&DataKey::ContadorReservas)
            .unwrap_or(0);
        let siguiente = id.checked_add(1).ok_or(Error::Desbordamiento)?;
        env.storage()
            .instance()
            .set(&DataKey::ContadorReservas, &siguiente);
        env.storage()
            .instance()
            .extend_ttl(TTL_UMBRAL, TTL_EXTENDER);

        let reserva = Reserva {
            comprador: comprador.clone(),
            productor,
            token: token.clone(),
            monto,
            fecha_limite,
            entregado_en: 0,
            estado: Estado::Reservado,
        };
        guardar(&env, id, &reserva);

        // Se guarda el estado antes de mover fondos (checks-effects-interactions).
        token::Client::new(&env, &token).transfer(
            &comprador,
            env.current_contract_address(),
            &monto,
        );

        Ok(id)
    }

    /// El productor declara que entregó el producto en la feria.
    pub fn marcar_entregado(env: Env, id: u64) -> Result<(), Error> {
        let mut reserva = cargar(&env, id)?;
        reserva.productor.require_auth();

        if reserva.estado != Estado::Reservado {
            return Err(Error::EstadoInvalido);
        }
        let ahora = env.ledger().timestamp();
        if ahora > reserva.fecha_limite {
            return Err(Error::FechaLimitePasada);
        }

        reserva.estado = Estado::Entregado;
        reserva.entregado_en = ahora;
        guardar(&env, id, &reserva);
        Ok(())
    }

    /// El comprador confirma que recibió el producto: se libera el pago al productor.
    pub fn confirmar_recepcion(env: Env, id: u64) -> Result<(), Error> {
        let mut reserva = cargar(&env, id)?;
        reserva.comprador.require_auth();

        if reserva.estado != Estado::Entregado {
            return Err(Error::EstadoInvalido);
        }

        reserva.estado = Estado::Pagado;
        guardar(&env, id, &reserva);
        pagar(&env, &reserva, &reserva.productor);
        Ok(())
    }

    /// El productor cobra si el comprador no confirmó dentro de `PLAZO_CONFIRMACION`.
    pub fn reclamar_pago(env: Env, id: u64) -> Result<(), Error> {
        let mut reserva = cargar(&env, id)?;
        reserva.productor.require_auth();

        if reserva.estado != Estado::Entregado {
            return Err(Error::EstadoInvalido);
        }
        let desde = reserva
            .entregado_en
            .checked_add(PLAZO_CONFIRMACION)
            .ok_or(Error::Desbordamiento)?;
        if env.ledger().timestamp() <= desde {
            return Err(Error::PlazoNoCumplido);
        }

        reserva.estado = Estado::Pagado;
        guardar(&env, id, &reserva);
        pagar(&env, &reserva, &reserva.productor);
        Ok(())
    }

    /// El comprador recupera su dinero si el productor nunca marcó la entrega
    /// y la fecha límite ya venció.
    pub fn reembolsar(env: Env, id: u64) -> Result<(), Error> {
        let mut reserva = cargar(&env, id)?;
        reserva.comprador.require_auth();

        if reserva.estado != Estado::Reservado {
            return Err(Error::EstadoInvalido);
        }
        if env.ledger().timestamp() <= reserva.fecha_limite {
            return Err(Error::FechaLimiteNoAlcanzada);
        }

        reserva.estado = Estado::Reembolsado;
        guardar(&env, id, &reserva);
        pagar(&env, &reserva, &reserva.comprador);
        Ok(())
    }

    /// Consulta de solo lectura.
    pub fn obtener_reserva(env: Env, id: u64) -> Result<Reserva, Error> {
        cargar(&env, id)
    }
}

fn cargar(env: &Env, id: u64) -> Result<Reserva, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Reserva(id))
        .ok_or(Error::ReservaNoExiste)
}

fn guardar(env: &Env, id: u64, reserva: &Reserva) {
    let key = DataKey::Reserva(id);
    env.storage().persistent().set(&key, reserva);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_UMBRAL, TTL_EXTENDER);
}

fn pagar(env: &Env, reserva: &Reserva, destino: &Address) {
    token::Client::new(env, &reserva.token).transfer(
        &env.current_contract_address(),
        destino,
        &reserva.monto,
    );
}
