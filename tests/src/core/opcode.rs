use tcg_core::opcode::*;
use tcg_core::types::Type;

#[test]
fn opcode_def_table_size() {
    assert_eq!(OPCODE_DEFS.len(), Opcode::Count as usize);
}

#[test]
fn opcode_def_lookup() {
    let def = Opcode::Add.def();
    assert_eq!(def.name, "add");
    assert_eq!(def.nb_oargs, 1);
    assert_eq!(def.nb_iargs, 2);
    assert_eq!(def.nb_cargs, 0);
    assert!(def.flags.contains(OpFlags::INT));
}

#[test]
fn opcode_def_nb_args() {
    assert_eq!(Opcode::Add.def().nb_args(), 3); // 1 out + 2 in
    assert_eq!(Opcode::Not.def().nb_args(), 2); // 1 out + 1 in
    assert_eq!(Opcode::BrCond.def().nb_args(), 4); // 0 out + 2 in + 2 const
    assert_eq!(Opcode::Nop.def().nb_args(), 0);
}

#[test]
fn opcode_int_polymorphic() {
    assert!(Opcode::Add.is_int_polymorphic());
    assert!(Opcode::Sub.is_int_polymorphic());
    assert!(Opcode::And.is_int_polymorphic());
    assert!(Opcode::Shl.is_int_polymorphic());
    assert!(!Opcode::ExtI32I64.is_int_polymorphic());
    assert!(!Opcode::Br.is_int_polymorphic());
    assert!(!Opcode::Nop.is_int_polymorphic());
}

#[test]
fn opcode_fixed_type() {
    assert_eq!(Opcode::ExtI32I64.fixed_type(), Some(Type::I64));
    assert_eq!(Opcode::ExtrlI64I32.fixed_type(), Some(Type::I32));
    assert_eq!(Opcode::Add.fixed_type(), None);
}

#[test]
fn opcode_control_flow_flags() {
    let br_def = Opcode::Br.def();
    assert!(br_def.flags.contains(OpFlags::BB_END));

    let brcond_def = Opcode::BrCond.def();
    assert!(brcond_def.flags.contains(OpFlags::BB_END));
    assert!(brcond_def.flags.contains(OpFlags::COND_BRANCH));

    let exit_def = Opcode::ExitTb.def();
    assert!(exit_def.flags.contains(OpFlags::BB_EXIT));
    assert!(exit_def.flags.contains(OpFlags::BB_END));

    let goto_def = Opcode::GotoTb.def();
    assert!(goto_def.flags.contains(OpFlags::BB_EXIT));
}

#[test]
fn opcode_side_effects() {
    assert!(Opcode::QemuLd.def().flags.contains(OpFlags::SIDE_EFFECTS));
    assert!(Opcode::QemuSt.def().flags.contains(OpFlags::SIDE_EFFECTS));
    assert!(Opcode::QemuLd.def().flags.contains(OpFlags::CALL_CLOBBER));
}

#[test]
fn opcode_carry_flags() {
    assert!(Opcode::AddCO.def().flags.contains(OpFlags::CARRY_OUT));
    assert!(!Opcode::AddCO.def().flags.contains(OpFlags::CARRY_IN));
    assert!(Opcode::AddCI.def().flags.contains(OpFlags::CARRY_IN));
    assert!(!Opcode::AddCI.def().flags.contains(OpFlags::CARRY_OUT));
    assert!(Opcode::AddCIO.def().flags.contains(OpFlags::CARRY_IN));
    assert!(Opcode::AddCIO.def().flags.contains(OpFlags::CARRY_OUT));
}

#[test]
fn opcode_names_unique() {
    let mut names: Vec<&str> = OPCODE_DEFS.iter().map(|d| d.name).collect();
    let len_before = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), len_before, "duplicate opcode names found");
}

#[test]
fn opcode_load_store_args() {
    // Host loads: 1 output, 1 input (base), 1 const (offset)
    let ld = Opcode::Ld.def();
    assert_eq!(ld.nb_oargs, 1);
    assert_eq!(ld.nb_iargs, 1);
    assert_eq!(ld.nb_cargs, 1);

    // Host stores: 0 output, 2 input (value, base), 1 const (offset)
    let st = Opcode::St.def();
    assert_eq!(st.nb_oargs, 0);
    assert_eq!(st.nb_iargs, 2);
    assert_eq!(st.nb_cargs, 1);
}
