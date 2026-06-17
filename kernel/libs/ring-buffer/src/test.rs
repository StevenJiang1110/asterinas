// SPDX-License-Identifier: MPL-2.0

use ostd::{
    mm::{VmReader, VmWriter},
    prelude::ktest,
};

use super::*;

#[ktest]
fn rb_basics() {
    let mut rb = RingBuffer::<i32>::new(4);
    rb.push(-100).unwrap();
    rb.push_slice(&[-1]).unwrap();
    assert_eq!(rb.len(), 2);

    let mut popped = [0i32; 2];
    rb.pop_slice(&mut popped).unwrap();
    assert_eq!(popped, [-100i32, -1]);
    assert!(rb.is_empty());

    rb.push_slice(&[i32::MAX, 1, -2, 100]).unwrap();
    assert!(rb.is_full());

    let popped = rb.pop();
    assert_eq!(popped, Some(i32::MAX));
    assert_eq!(rb.free_len(), 1);

    let mut popped = [0i32; 3];
    rb.pop_slice(&mut popped).unwrap();
    assert_eq!(popped, [1i32, -2, 100]);
    assert_eq!(rb.free_len(), 4);
}

#[ktest]
fn rb_write_read_one() {
    let rb = RingBuffer::<u8>::new(1);

    let (mut prod, mut cons) = rb.split();
    assert_eq!(prod.capacity(), 1);
    assert_eq!(cons.capacity(), 1);

    assert!(cons.pop().is_none());
    assert!(prod.push(1).is_some());
    assert!(prod.is_full());

    assert!(prod.push(2).is_none());
    assert!(prod.push_slice(&[2]).is_none());
    assert_eq!(cons.pop().unwrap(), 1u8);
    assert!(cons.is_empty());
}

#[ktest]
fn rb_write_read_all() {
    let rb = RingBuffer::<u8>::new(4 * PAGE_SIZE);
    assert_eq!(rb.capacity(), 4 * PAGE_SIZE);

    let (mut prod, mut cons) = rb.split();
    prod.push(u8::MIN).unwrap();
    assert_eq!(cons.pop().unwrap(), u8::MIN);

    prod.push_slice(&[u8::MAX]).unwrap();
    let mut popped = [0u8];
    cons.pop_slice(&mut popped).unwrap();
    assert_eq!(popped, [u8::MAX]);

    let step = 128;
    let mut input = alloc::vec![0u8; step];
    for i in (0..4 * PAGE_SIZE).step_by(step) {
        input.fill(i as _);
        prod.push_slice(&input).unwrap();
    }
    assert!(cons.is_full());

    let mut output = alloc::vec![0u8; step];
    for i in (0..4 * PAGE_SIZE).step_by(step) {
        cons.pop_slice(&mut output).unwrap();
        assert_eq!(output[0], i as u8);
        assert_eq!(output[step - 1], i as u8);
    }
    assert!(prod.is_empty());
}

#[ktest]
fn rb_write_read_one_with_vm_io() {
    let rb = RingBuffer::<u8>::new(1);

    let (mut prod, mut cons) = rb.split();

    let input = [u8::MAX];
    assert_eq!(
        prod.write_fallible(&mut reader_from(input.as_slice()))
            .unwrap(),
        1
    );
    assert_eq!(
        prod.write_fallible(&mut reader_from(input.as_slice()))
            .unwrap(),
        0
    );
    assert_eq!(prod.len(), 1);

    let mut output = [0u8];
    assert_eq!(
        cons.read_fallible(&mut writer_from(output.as_mut_slice()))
            .unwrap(),
        1
    );
    assert_eq!(
        cons.read_fallible(&mut writer_from(output.as_mut_slice()))
            .unwrap(),
        0
    );
    assert_eq!(cons.free_len(), 1);

    assert_eq!(output, input);
}

#[ktest]
fn rb_write_read_all_with_vm_io() {
    let rb = RingBuffer::<u8>::new(4 * PAGE_SIZE);
    assert_eq!(rb.capacity(), 4 * PAGE_SIZE);

    let (mut prod, mut cons) = rb.split();

    let step = 128;
    let mut input = alloc::vec![0u8; step];
    for i in (0..4 * PAGE_SIZE).step_by(step) {
        input.fill(i as _);
        let write_len = prod
            .write_fallible(&mut reader_from(input.as_slice()))
            .unwrap();
        assert_eq!(write_len, step);
    }
    assert!(cons.is_full());

    let mut output = alloc::vec![0u8; step];
    for i in (0..4 * PAGE_SIZE).step_by(step) {
        let read_len = cons
            .read_fallible(&mut writer_from(output.as_mut_slice()))
            .unwrap();
        assert_eq!(read_len, step);
        assert_eq!(output[0], i as u8);
        assert_eq!(output[step - 1], i as u8);
    }
    assert!(prod.is_empty());
}

fn reader_from(buf: &[u8]) -> VmReader<'_> {
    VmReader::from(buf).to_fallible()
}

fn writer_from(buf: &mut [u8]) -> VmWriter<'_> {
    VmWriter::from(buf).to_fallible()
}
