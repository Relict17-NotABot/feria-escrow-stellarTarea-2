#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::{StellarAssetClient, TokenClient},
    Address, Env, IntoVal,
};

const MONTO: i128 = 50_000_000; // 5 XLM expresados en stroops
const T0: u64 = 1_000;
const FECHA_LIMITE: u64 = 10_000;

struct Setup<'a> {
    env: Env,
    contrato: Address,
    client: FeriaContractClient<'a>,
    token: TokenClient<'a>,
    comprador: Address,
    productor: Address,
}

fn setup<'a>() -> Setup<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(T0);

    let emisor = Address::generate(&env);
    let sac = env.register_stellar_asset_contract_v2(emisor);
    let token_addr = sac.address();
    let token = TokenClient::new(&env, &token_addr);

    let comprador = Address::generate(&env);
    let productor = Address::generate(&env);
    StellarAssetClient::new(&env, &token_addr).mint(&comprador, &(MONTO * 10));

    let contrato = env.register(FeriaContract, ());
    let client = FeriaContractClient::new(&env, &contrato);

    Setup {
        env,
        contrato,
        client,
        token,
        comprador,
        productor,
    }
}

fn reservar(s: &Setup) -> u64 {
    s.client.reservar(
        &s.comprador,
        &s.productor,
        &s.token.address,
        &MONTO,
        &FECHA_LIMITE,
    )
}

fn avanzar_a(s: &Setup, t: u64) {
    s.env.ledger().set_timestamp(t);
}

// ---------- Flujos exitosos ----------

#[test]
fn flujo_completo_paga_al_productor() {
    let s = setup();
    let id = reservar(&s);

    assert_eq!(s.token.balance(&s.comprador), MONTO * 9);
    assert_eq!(s.token.balance(&s.contrato), MONTO);
    assert_eq!(s.client.obtener_reserva(&id).estado, Estado::Reservado);

    s.client.marcar_entregado(&id);
    assert_eq!(s.client.obtener_reserva(&id).estado, Estado::Entregado);

    s.client.confirmar_recepcion(&id);
    assert_eq!(s.client.obtener_reserva(&id).estado, Estado::Pagado);
    assert_eq!(s.token.balance(&s.productor), MONTO);
    assert_eq!(s.token.balance(&s.contrato), 0);
}

#[test]
fn reembolso_si_no_hubo_entrega() {
    let s = setup();
    let id = reservar(&s);

    avanzar_a(&s, FECHA_LIMITE + 1);
    s.client.reembolsar(&id);

    assert_eq!(s.client.obtener_reserva(&id).estado, Estado::Reembolsado);
    assert_eq!(s.token.balance(&s.comprador), MONTO * 10);
    assert_eq!(s.token.balance(&s.contrato), 0);
}

#[test]
fn productor_reclama_si_comprador_no_confirma() {
    let s = setup();
    let id = reservar(&s);
    s.client.marcar_entregado(&id);

    avanzar_a(&s, T0 + PLAZO_CONFIRMACION + 1);
    s.client.reclamar_pago(&id);

    assert_eq!(s.client.obtener_reserva(&id).estado, Estado::Pagado);
    assert_eq!(s.token.balance(&s.productor), MONTO);
}

#[test]
fn ids_son_secuenciales_y_reservas_independientes() {
    let s = setup();
    let a = reservar(&s);
    let b = reservar(&s);
    assert_eq!(a, 0);
    assert_eq!(b, 1);

    s.client.marcar_entregado(&a);
    assert_eq!(s.client.obtener_reserva(&a).estado, Estado::Entregado);
    assert_eq!(s.client.obtener_reserva(&b).estado, Estado::Reservado);
}

// ---------- Validaciones de reservar ----------

#[test]
fn reservar_rechaza_monto_cero_o_negativo() {
    let s = setup();
    for monto in [0i128, -1] {
        let r = s.client.try_reservar(
            &s.comprador,
            &s.productor,
            &s.token.address,
            &monto,
            &FECHA_LIMITE,
        );
        assert_eq!(r, Err(Ok(Error::MontoInvalido)));
    }
}

#[test]
fn reservar_rechaza_fecha_pasada() {
    let s = setup();
    let r = s
        .client
        .try_reservar(&s.comprador, &s.productor, &s.token.address, &MONTO, &T0);
    assert_eq!(r, Err(Ok(Error::FechaInvalida)));
}

#[test]
fn reservar_rechaza_comprador_igual_a_productor() {
    let s = setup();
    let r = s.client.try_reservar(
        &s.comprador,
        &s.comprador,
        &s.token.address,
        &MONTO,
        &FECHA_LIMITE,
    );
    assert_eq!(r, Err(Ok(Error::MismaPersona)));
}

#[test]
fn reserva_inexistente_da_error_claro() {
    let s = setup();
    assert_eq!(
        s.client.try_obtener_reserva(&99),
        Err(Ok(Error::ReservaNoExiste))
    );
    assert_eq!(
        s.client.try_marcar_entregado(&99),
        Err(Ok(Error::ReservaNoExiste))
    );
}

// ---------- Estados y tiempos ----------

#[test]
fn no_se_puede_pagar_dos_veces() {
    let s = setup();
    let id = reservar(&s);
    s.client.marcar_entregado(&id);
    s.client.confirmar_recepcion(&id);

    assert_eq!(
        s.client.try_confirmar_recepcion(&id),
        Err(Ok(Error::EstadoInvalido))
    );
    avanzar_a(&s, T0 + PLAZO_CONFIRMACION + 1);
    assert_eq!(
        s.client.try_reclamar_pago(&id),
        Err(Ok(Error::EstadoInvalido))
    );
    assert_eq!(s.token.balance(&s.productor), MONTO);
}

#[test]
fn no_se_puede_reembolsar_dos_veces() {
    let s = setup();
    let id = reservar(&s);
    avanzar_a(&s, FECHA_LIMITE + 1);
    s.client.reembolsar(&id);
    assert_eq!(s.client.try_reembolsar(&id), Err(Ok(Error::EstadoInvalido)));
    assert_eq!(s.token.balance(&s.comprador), MONTO * 10);
}

#[test]
fn no_se_reembolsa_antes_de_la_fecha_limite() {
    let s = setup();
    let id = reservar(&s);
    assert_eq!(
        s.client.try_reembolsar(&id),
        Err(Ok(Error::FechaLimiteNoAlcanzada))
    );
    avanzar_a(&s, FECHA_LIMITE); // en el límite exacto todavía no vence
    assert_eq!(
        s.client.try_reembolsar(&id),
        Err(Ok(Error::FechaLimiteNoAlcanzada))
    );
}

#[test]
fn no_se_reembolsa_si_ya_hubo_entrega() {
    let s = setup();
    let id = reservar(&s);
    s.client.marcar_entregado(&id);
    avanzar_a(&s, FECHA_LIMITE + 1);
    assert_eq!(s.client.try_reembolsar(&id), Err(Ok(Error::EstadoInvalido)));
}

#[test]
fn no_se_marca_entrega_despues_de_la_fecha_limite() {
    let s = setup();
    let id = reservar(&s);
    avanzar_a(&s, FECHA_LIMITE + 1);
    assert_eq!(
        s.client.try_marcar_entregado(&id),
        Err(Ok(Error::FechaLimitePasada))
    );
}

#[test]
fn no_se_reclama_pago_antes_del_plazo() {
    let s = setup();
    let id = reservar(&s);
    s.client.marcar_entregado(&id);
    assert_eq!(
        s.client.try_reclamar_pago(&id),
        Err(Ok(Error::PlazoNoCumplido))
    );
    avanzar_a(&s, T0 + PLAZO_CONFIRMACION); // en el límite exacto todavía no vence
    assert_eq!(
        s.client.try_reclamar_pago(&id),
        Err(Ok(Error::PlazoNoCumplido))
    );
}

#[test]
fn no_se_confirma_recepcion_sin_entrega() {
    let s = setup();
    let id = reservar(&s);
    assert_eq!(
        s.client.try_confirmar_recepcion(&id),
        Err(Ok(Error::EstadoInvalido))
    );
}

// ---------- Autorización (sin mock_all_auths: firma solo quien se indica) ----------

/// Intenta ejecutar `fn_name(id)` firmando únicamente `firmante`.
fn firma_solo(s: &Setup, firmante: &Address, fn_name: &'static str, id: u64) {
    s.env.mock_auths(&[MockAuth {
        address: firmante,
        invoke: &MockAuthInvoke {
            contract: &s.contrato,
            fn_name,
            args: (id,).into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
}

#[test]
fn solo_el_productor_puede_marcar_entrega() {
    let s = setup();
    let id = reservar(&s);
    let intruso = Address::generate(&s.env);

    firma_solo(&s, &intruso, "marcar_entregado", id);
    assert!(s.client.try_marcar_entregado(&id).is_err());
    assert_eq!(s.client.obtener_reserva(&id).estado, Estado::Reservado);

    // El comprador tampoco puede marcar la entrega por el productor.
    firma_solo(&s, &s.comprador, "marcar_entregado", id);
    assert!(s.client.try_marcar_entregado(&id).is_err());

    firma_solo(&s, &s.productor, "marcar_entregado", id);
    assert!(s.client.try_marcar_entregado(&id).is_ok());
}

#[test]
fn solo_el_comprador_puede_confirmar_recepcion() {
    let s = setup();
    let id = reservar(&s);
    s.client.marcar_entregado(&id);

    // El productor no puede confirmar por el comprador.
    firma_solo(&s, &s.productor, "confirmar_recepcion", id);
    assert!(s.client.try_confirmar_recepcion(&id).is_err());
    assert_eq!(s.token.balance(&s.productor), 0);

    firma_solo(&s, &s.comprador, "confirmar_recepcion", id);
    assert!(s.client.try_confirmar_recepcion(&id).is_ok());
    assert_eq!(s.token.balance(&s.productor), MONTO);
}

#[test]
fn solo_el_comprador_puede_reembolsar() {
    let s = setup();
    let id = reservar(&s);
    avanzar_a(&s, FECHA_LIMITE + 1);

    let intruso = Address::generate(&s.env);
    firma_solo(&s, &intruso, "reembolsar", id);
    assert!(s.client.try_reembolsar(&id).is_err());
    firma_solo(&s, &s.productor, "reembolsar", id);
    assert!(s.client.try_reembolsar(&id).is_err());
    assert_eq!(s.token.balance(&s.contrato), MONTO);
}

#[test]
fn solo_el_productor_puede_reclamar_pago() {
    let s = setup();
    let id = reservar(&s);
    s.client.marcar_entregado(&id);
    avanzar_a(&s, T0 + PLAZO_CONFIRMACION + 1);

    let intruso = Address::generate(&s.env);
    firma_solo(&s, &intruso, "reclamar_pago", id);
    assert!(s.client.try_reclamar_pago(&id).is_err());
    assert_eq!(s.token.balance(&s.contrato), MONTO);
}

#[test]
fn reservar_exige_firma_del_comprador() {
    let s = setup();
    let intruso = Address::generate(&s.env);
    // Un tercero firma, pero los fondos salen del comprador: debe fallar.
    s.env.mock_auths(&[MockAuth {
        address: &intruso,
        invoke: &MockAuthInvoke {
            contract: &s.contrato,
            fn_name: "reservar",
            args: (
                &s.comprador,
                &s.productor,
                &s.token.address,
                MONTO,
                FECHA_LIMITE,
            )
                .into_val(&s.env),
            sub_invokes: &[],
        },
    }]);
    let r = s.client.try_reservar(
        &s.comprador,
        &s.productor,
        &s.token.address,
        &MONTO,
        &FECHA_LIMITE,
    );
    assert!(r.is_err());
    assert_eq!(s.token.balance(&s.comprador), MONTO * 10);
}
