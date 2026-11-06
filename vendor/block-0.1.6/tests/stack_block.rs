extern crate block;

#[test]
fn stack_block_can_be_invoked_copied_and_released() {
    let captured = String::from("captured");
    let block = block::ConcreteBlock::new(move |value: i32| value + captured.len() as i32);
    assert_eq!(unsafe { block.call((5,)) }, 13);
    let copied = block.copy();
    let retained = copied.clone();
    drop(copied);
    assert_eq!(unsafe { retained.call((7,)) }, 15);
}
