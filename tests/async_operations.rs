#![cfg(feature = "async")]
use rtrb::RingBuffer;
use rtrb::*;

#[tokio::test]
async fn exceed_capacity(){
    let (mut p, mut c) = RingBuffer::<i32>::new_async(1);
    assert!(matches!(p.write_chunk_async(2).await.unwrap_err(),AsyncChunkError::ExceedCapacity(1))); 
    assert!(matches!(c.read_chunk_async(2).await.unwrap_err(),AsyncChunkError::ExceedCapacity(1)));
}

#[tokio::test]
async fn read_async(){
    let (mut p, mut c) = RingBuffer::new_async(1);
    let value = 2021;
    let consume = tokio::spawn(async move{
        let chunk = c.read_chunk_async(1).await.unwrap();
        chunk.into_iter().next().unwrap()
    });
    p.push(value).unwrap();
    assert_eq!(consume.await.unwrap(),value);
    
}
#[tokio::test]
async fn write_async(){
    let (mut p, mut c) = RingBuffer::new_async(1);
    p.push(0).unwrap();
    let value = 2021;
    let produce = tokio::spawn(async move{
        let chunk = p.write_chunk_uninit_async(1).await.unwrap();
        chunk.fill_from_iter(core::iter::once(value));
    });

    assert_eq!(c.pop().unwrap(),0);
    produce.await.unwrap();
    assert_eq!(c.pop().unwrap(),value);
    
}
#[tokio::test]
async fn abandon(){
    let (p, mut c) = RingBuffer::<i32>::new_async(1);
    tokio::join!(async move{
        assert!(matches!(c.read_chunk_async(1).await.unwrap_err(),AsyncChunkError::TooFewSlotsAndAbandoned(0)));
        assert!(matches!(c.read_chunk_async(1).await.unwrap_err(),AsyncChunkError::TooFewSlotsAndAbandoned(0)));
    },async move{drop(p)});
}
#[tokio::test]
async fn deadlock(){
    let (mut p, mut c) = RingBuffer::<i32>::new_async(2);
    p.push(1).unwrap();
    tokio::join!(async move{
        c.read_chunk_async(2).await.unwrap();
    },async move{
        assert!(matches!(p.write_chunk_uninit_async(2).await.unwrap_err(),AsyncChunkError::WillDeadlock(1,1)));
        p.push(2).unwrap();
    });
}